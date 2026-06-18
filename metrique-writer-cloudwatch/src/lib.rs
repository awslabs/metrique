//! CloudWatch Logs `PutLogEvents` (EMF) backend for metrique.
//!
//! This crate provides [`CwLogsStream`], an [`EntryIoStream`] implementation that serializes
//! metric entries as EMF JSON and submits them directly to CloudWatch Logs via `PutLogEvents`.
//!
//! # Architecture
//!
//! ```text
//! App threads → BackgroundQueue (metrique) → std thread calls CwLogsStream::next(&entry)
//!   → EMF serialize (reuses metrique-writer-format-emf)
//!   → batch until size/count limit hit: try_send(batch) over bounded channel
//!   → single async task: recv batch → client.put_log_events (async HTTP)
//! ```
//!
//! # Runtime
//!
//! The `tokio_runtime` feature is enabled by default and provides the default
//! [`TaskSpawner::tokio()`] spawner. With `--no-default-features`, callers must
//! provide a custom [`TaskSpawner`] with [`CwLogsStream::builder()`]. Custom
//! spawners must detach or otherwise keep the submitted future running after
//! the spawn callback returns.
//!
//! For non-Tokio executors, configure the AWS client with a compatible
//! [`AsyncSleep`](aws_sdk_cloudwatchlogs::config::AsyncSleep). SDK timeouts and
//! retry delays use that sleep hook.
//!
//! # Backpressure
//!
//! When the submission channel is full (CloudWatch Logs is slow or down), batches are
//! dropped at submission time with a rate-limited warning. Drops are reported through
//! [`CwLogsStreamEvent::BatchDropped`]. Once the channel drains, normal submission resumes.
//!
//! # Shutdown
//!
//! [`CwLogsStream::builder()`] returns `(CwLogsStream, CwLogsStreamHandle)`. Call
//! [`CwLogsStreamHandle::shutdown()`] to await drain of in-flight batches.
//!
//! # Example
//!
//! ```rust,ignore
//! # async fn example() {
//! use metrique_writer_cloudwatch::{CwLogsStream, CwLogsStreamConfig};
//! use metrique_writer_core::EntryIoStream;
//!
//! let sdk_config = aws_config::load_from_env().await;
//! let client = aws_sdk_cloudwatchlogs::Client::new(&sdk_config);
//!
//! let (stream, handle) = CwLogsStream::builder()
//!     .client(client)
//!     .log_group_name("/my-app/metrics".to_string())
//!     .log_stream_name("host-1".to_string())
//!     .namespace("MyApp".to_string())
//!     .default_dimensions(vec![vec![]])
//!     .build();
//!
//! // Pass `stream` to metrique's BackgroundQueue or ServiceMetrics::attach_to_stream()
//! // ...
//!
//! // Graceful shutdown:
//! handle.shutdown().await;
//! # }
//! ```

use std::future::Future;
use std::io;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_sdk_cloudwatchlogs::Client as CloudWatchLogsClient;
use aws_sdk_cloudwatchlogs::error::SdkError;
use aws_sdk_cloudwatchlogs::operation::create_log_group::CreateLogGroupError;
use aws_sdk_cloudwatchlogs::operation::create_log_stream::CreateLogStreamError;
use aws_sdk_cloudwatchlogs::types::InputLogEvent;
use bon::bon;
use metrique_writer_core::format::Format;
use metrique_writer_core::{Entry, EntryIoStream, IoStreamError};
use metrique_writer_format_emf::Emf;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

/// Maximum payload size for a single PutLogEvents request (1 MB).
const MAX_BATCH_BYTES: usize = 1_048_576;
/// Maximum size of a single log event message (1 MB).
const MAX_EVENT_BYTES: usize = 1_048_576;
/// Overhead per log event (timestamp + framing).
const EVENT_OVERHEAD_BYTES: usize = 26;
/// Maximum number of log events per PutLogEvents request.
const MAX_BATCH_EVENTS: usize = 10_000;

/// Configuration for [`CwLogsStream`].
#[derive(Debug, Clone, bon::Builder)]
pub struct CwLogsStreamConfig {
    /// Capacity of the tokio mpsc channel between the collection thread and the
    /// submission task. Default: 5.
    #[builder(default = NonZeroUsize::new(5).unwrap())]
    pub channel_capacity: NonZeroUsize,

    /// Whether to auto-create the log group and stream if they don't exist. Default: true.
    #[builder(default = true)]
    pub auto_create: bool,
}

impl Default for CwLogsStreamConfig {
    fn default() -> Self {
        Self {
            channel_capacity: NonZeroUsize::new(5).unwrap(),
            auto_create: true,
        }
    }
}

/// Events emitted by [`CwLogsStream`] for observability.
#[derive(Debug)]
#[non_exhaustive]
pub enum CwLogsStreamEvent {
    /// A batch was dropped due to backpressure (submission channel full).
    #[non_exhaustive]
    BatchDropped {
        /// Number of events in the dropped batch.
        event_count: usize,
    },
    /// A PutLogEvents submission failed.
    #[non_exhaustive]
    SubmissionFailed,
    /// An entry was dropped because the stream is shut down.
    #[non_exhaustive]
    EntryDroppedAfterShutdown,
    /// The EMF formatter produced invalid UTF-8 (indicates a bug in the formatter).
    #[non_exhaustive]
    InvalidUtf8,
    /// A single event exceeded the per-event size limit (1 MB) and was dropped.
    #[non_exhaustive]
    EventOversized,
}

/// Observer for [`CwLogsStreamEvent`]s. Implement this to collect internal metrics.
pub trait CwLogsStreamObserver: Send + Sync + std::fmt::Debug + 'static {
    /// Called when an event occurs.
    fn observe(&self, event: &CwLogsStreamEvent);
}

/// Default no-op observer.
#[derive(Debug, Default)]
pub struct NoOpObserver;
impl CwLogsStreamObserver for NoOpObserver {
    fn observe(&self, _event: &CwLogsStreamEvent) {}
}

/// Type alias for a task spawner function.
// TODO: Consider upstreaming to metrique-writer-core as a shared utility for async EntryIoStream backends.
pub type TaskSpawnerFn =
    Box<dyn Fn(Pin<Box<dyn Future<Output = ()> + Send>>) + Send + Sync + 'static>;

/// A wrapper around a task spawner function.
pub struct TaskSpawner(TaskSpawnerFn);

impl std::fmt::Debug for TaskSpawner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskSpawner").finish_non_exhaustive()
    }
}

impl TaskSpawner {
    /// Create a new TaskSpawner with a custom spawn function.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(Pin<Box<dyn Future<Output = ()> + Send>>) + Send + Sync + 'static,
    {
        Self(Box::new(f))
    }

    /// Spawn a task.
    pub fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        (self.0)(future);
    }

    /// Create a task spawner that uses `tokio::spawn`.
    #[cfg(feature = "tokio_runtime")]
    pub fn tokio() -> Self {
        Self::new(|future| {
            tokio::spawn(future);
        })
    }
}

/// An [`EntryIoStream`] that serializes entries as EMF JSON and submits them
/// to CloudWatch Logs via `PutLogEvents` in a background async task.
pub struct CwLogsStream {
    emf: Emf,
    log_group_name: String,
    log_stream_name: String,
    batch: Vec<InputLogEvent>,
    batch_bytes: usize,
    tx: Option<mpsc::Sender<WorkerCommand>>,
    observer: Arc<dyn CwLogsStreamObserver>,
    submit_task_spawned: bool,
}

impl std::fmt::Debug for CwLogsStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CwLogsStream")
            .field("log_group_name", &self.log_group_name)
            .field("log_stream_name", &self.log_stream_name)
            .field("batch_len", &self.batch.len())
            .field("batch_bytes", &self.batch_bytes)
            .field("shutdown", &self.tx.is_none())
            .finish()
    }
}

enum WorkerCommand {
    PutLogEvents(Vec<InputLogEvent>),
    Shutdown(oneshot::Sender<()>),
}

/// Handle for async shutdown of [`CwLogsStream`].
///
/// Call [`shutdown()`](Self::shutdown) to wait until all in-flight PutLogEvents
/// batches finish. If the stream has already been dropped, `shutdown()` returns
/// immediately (the worker drains buffered payloads on its own when the channel
/// closes).
///
// TODO: this stinks, ideally we can register our entry io stream as wanting to know about
// async flushes, in BackgroundQueue's async flush impl. That way we don't have to vend
// a separate channel.
#[derive(Debug)]
pub struct CwLogsStreamHandle(Option<oneshot::Sender<oneshot::Sender<()>>>);

impl CwLogsStreamHandle {
    /// Async flush of remaining PutLogEvents batches. The submit loop drains
    /// remaining payloads, then signals completion.
    ///
    /// After this returns, the stream detects the closed channel on the
    /// next `enqueue_batch` and becomes inert.
    pub async fn shutdown(self) {
        let Some(shutdown_tx) = self.0 else { return };
        let (tx, rx) = oneshot::channel();
        if shutdown_tx.send(tx).is_ok() {
            if rx.await.is_err() {
                tracing::warn!("CwLogsStream closed while waiting on shutdown response");
            }
        } else {
            tracing::warn!("CwLogsStream already shut down when shutdown() was called");
        }
    }
}

#[bon]
impl CwLogsStream {
    /// Create a new [`CwLogsStream`].
    ///
    /// # Panics
    ///
    /// Panics if the default `tokio_runtime` spawner, or
    /// [`TaskSpawner::tokio()`], is used outside of a Tokio runtime context.
    /// A custom [`TaskSpawner`] may also panic according to its implementation.
    #[builder]
    pub fn new(
        client: CloudWatchLogsClient,
        log_group_name: String,
        log_stream_name: String,
        namespace: String,
        #[builder(default = vec![vec![]])] default_dimensions: Vec<Vec<String>>,
        #[builder(default)] config: CwLogsStreamConfig,
        #[cfg_attr(feature = "tokio_runtime", builder(default = TaskSpawner::tokio()))]
        task_spawner: TaskSpawner,
        observer: Option<Box<dyn CwLogsStreamObserver>>,
    ) -> (Self, CwLogsStreamHandle) {
        let (tx, rx) = mpsc::channel(config.channel_capacity.get());
        let observer: Arc<dyn CwLogsStreamObserver> =
            Arc::from(observer.unwrap_or_else(|| Box::new(NoOpObserver)));
        let spawner = task_spawner;

        spawner.spawn(Box::pin(submit_loop(
            rx,
            client.clone(),
            log_group_name.clone(),
            log_stream_name.clone(),
            config.auto_create,
            observer.clone(),
        )));

        // Shutdown relay: when the handle sends a shutdown signal, forward it to the worker.
        // TODO: this stinks, ideally we can register our entry io stream as wanting to know about
        // async flushes, in BackgroundQueue's async flush impl. That way we don't have to vend
        // a separate channel. We also want the worker to flush all in-flight batches during
        // attach handle drop / explicit shutdown automatically.
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let weak_tx = tx.downgrade();
        spawner.spawn(Box::pin(async move {
            if let Ok(response_tx) = shutdown_rx.await {
                if let Some(sender) = weak_tx.upgrade() {
                    let _ = sender.send(WorkerCommand::Shutdown(response_tx)).await;
                } else {
                    let _ = response_tx.send(());
                }
            }
        }));

        let handle = CwLogsStreamHandle(Some(shutdown_tx));
        let emf = Emf::builder(namespace, default_dimensions).build();

        (
            Self {
                emf,
                log_group_name,
                log_stream_name,
                batch: Vec::new(),
                batch_bytes: 0,
                tx: Some(tx),
                observer,
                submit_task_spawned: true,
            },
            handle,
        )
    }

    fn enqueue_batch(&mut self) {
        let Some(tx) = self.tx.as_ref() else { return };
        let batch = std::mem::take(&mut self.batch);
        let count = batch.len();
        self.batch_bytes = 0;

        match tx.try_send(WorkerCommand::PutLogEvents(batch)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                // TODO: drop oldest batch instead of newest for consistency with BackgroundQueue.
                self.observer
                    .observe(&CwLogsStreamEvent::BatchDropped { event_count: count });
                warn!("CloudWatch Logs submission channel full, dropping batch of {count} events");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // TODO: clean up shutdown signaling when upstreaming to metrique —
                // ideally BackgroundQueue would notify the stream directly.
                self.tx.take();
            }
        }
    }

    fn batch_would_exceed_limits(&self, event_bytes: usize) -> bool {
        self.batch.len() >= MAX_BATCH_EVENTS
            || self.batch_bytes + event_bytes + EVENT_OVERHEAD_BYTES > MAX_BATCH_BYTES
    }
}

impl Drop for CwLogsStream {
    fn drop(&mut self) {
        if !self.batch.is_empty() {
            self.enqueue_batch();
        }
        if self.submit_task_spawned && self.tx.is_some() {
            tracing::warn!(
                "CwLogsStream dropped without calling shutdown() — \
                 in-flight batches will drain in the background but \
                 completion cannot be awaited"
            );
        }
        self.tx.take();
        self.submit_task_spawned = false;
    }
}

impl EntryIoStream for CwLogsStream {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        if self.tx.is_none() {
            warn!("CwLogsStream::next() called after shutdown, entry dropped");
            self.observer
                .observe(&CwLogsStreamEvent::EntryDroppedAfterShutdown);
            return Ok(());
        }

        // Serialize the entry to EMF JSON. The formatter may produce multiple
        // newline-separated JSON objects (one per dimension set).
        let mut buf = Vec::with_capacity(1024);
        self.emf.format(entry, &mut buf)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let output = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(_) => {
                warn!("CwLogsStream: EMF formatter produced invalid UTF-8, dropping entry");
                self.observer.observe(&CwLogsStreamEvent::InvalidUtf8);
                return Ok(());
            }
        };

        // Each line is a separate EMF JSON record → separate log event.
        for line in output.lines() {
            if line.is_empty() {
                continue;
            }
            let event_bytes = line.len();

            // Skip events that exceed the per-event size limit (1 MB).
            if event_bytes > MAX_EVENT_BYTES {
                warn!(
                    "CwLogsStream: skipping oversized event ({event_bytes} bytes, limit {MAX_EVENT_BYTES})"
                );
                self.observer.observe(&CwLogsStreamEvent::EventOversized);
                continue;
            }

            if !self.batch.is_empty() && self.batch_would_exceed_limits(event_bytes) {
                self.enqueue_batch();
            }

            if let Ok(event) = InputLogEvent::builder()
                .message(line)
                .timestamp(now)
                .build()
            {
                self.batch_bytes += event_bytes + EVENT_OVERHEAD_BYTES;
                self.batch.push(event);
            }
        }

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.batch.is_empty() {
            self.enqueue_batch();
        }
        Ok(())
    }
}

async fn submit_loop(
    mut rx: mpsc::Receiver<WorkerCommand>,
    client: CloudWatchLogsClient,
    log_group_name: String,
    log_stream_name: String,
    auto_create: bool,
    observer: Arc<dyn CwLogsStreamObserver>,
) {
    if auto_create {
        create_log_resources(&client, &log_group_name, &log_stream_name).await;
    }

    while let Some(command) = rx.recv().await {
        match command {
            WorkerCommand::PutLogEvents(events) => {
                submit_batch(
                    &client,
                    &log_group_name,
                    &log_stream_name,
                    events,
                    &observer,
                )
                .await;
            }
            WorkerCommand::Shutdown(response_tx) => {
                // Drain remaining batches.
                rx.close();
                while let Some(cmd) = rx.recv().await {
                    if let WorkerCommand::PutLogEvents(events) = cmd {
                        submit_batch(
                            &client,
                            &log_group_name,
                            &log_stream_name,
                            events,
                            &observer,
                        )
                        .await;
                    }
                }
                let _ = response_tx.send(());
                break;
            }
        }
    }
}

async fn submit_batch(
    client: &CloudWatchLogsClient,
    log_group_name: &str,
    log_stream_name: &str,
    events: Vec<InputLogEvent>,
    observer: &Arc<dyn CwLogsStreamObserver>,
) {
    let result = client
        .put_log_events()
        .log_group_name(log_group_name)
        .log_stream_name(log_stream_name)
        .set_log_events(Some(events))
        .send()
        .await;

    match result {
        Err(e) => {
            warn!("CloudWatch Logs PutLogEvents failed: {e}");
            observer.observe(&CwLogsStreamEvent::SubmissionFailed);
        }
        Ok(output) => {
            if let Some(rejected) = output.rejected_log_events_info() {
                warn!("CloudWatch Logs rejected events: {rejected:?}");
            }
        }
    }
}

// Best-effort creation — ignore "already exists" errors.
async fn create_log_resources(
    client: &CloudWatchLogsClient,
    log_group_name: &str,
    log_stream_name: &str,
) {
    match client
        .create_log_group()
        .log_group_name(log_group_name)
        .send()
        .await
    {
        Ok(_) => debug!("Created log group: {log_group_name}"),
        Err(SdkError::ServiceError(ref e))
            if matches!(
                e.err(),
                CreateLogGroupError::ResourceAlreadyExistsException(_)
            ) =>
        {
            debug!("Log group already exists: {log_group_name}");
        }
        Err(e) => {
            warn!("Failed to create log group {log_group_name}: {e}");
        }
    }

    match client
        .create_log_stream()
        .log_group_name(log_group_name)
        .log_stream_name(log_stream_name)
        .send()
        .await
    {
        Ok(_) => debug!("Created log stream: {log_stream_name}"),
        Err(SdkError::ServiceError(ref e))
            if matches!(
                e.err(),
                CreateLogStreamError::ResourceAlreadyExistsException(_)
            ) =>
        {
            debug!("Log stream already exists: {log_stream_name}");
        }
        Err(e) => {
            warn!("Failed to create log stream {log_stream_name}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_cloudwatchlogs::config::timeout::TimeoutConfig;
    use aws_sdk_cloudwatchlogs::config::{AsyncSleep, BehaviorVersion, Credentials, Region, Sleep};
    use aws_smithy_http_client::test_util::infallible_client_fn;
    use aws_smithy_runtime_api::client::http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    };
    use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
    use aws_smithy_runtime_api::shared::IntoShared;
    use aws_smithy_types::body::SdkBody;
    use metrique_writer::sink::BackgroundQueueBuilder;
    use metrique_writer_core::{AnyEntrySink, EntryWriter};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;
    use tracing_test::traced_test;

    fn success_response() -> http::Response<SdkBody> {
        http::Response::builder()
            .status(200)
            .body(SdkBody::from("{}"))
            .unwrap()
    }

    fn error_response() -> http::Response<SdkBody> {
        http::Response::builder()
            .status(503)
            .body(SdkBody::from("Service Unavailable"))
            .unwrap()
    }

    #[derive(Debug)]
    struct InstantSleep;

    impl AsyncSleep for InstantSleep {
        fn sleep(&self, _duration: Duration) -> Sleep {
            Sleep::new(std::future::ready(()))
        }
    }

    fn test_client_success() -> CloudWatchLogsClient {
        test_client(infallible_client_fn(|_req| success_response()))
    }

    fn test_client_failing() -> CloudWatchLogsClient {
        test_client(infallible_client_fn(|_req| error_response()))
    }

    fn test_client(http_client: impl HttpClient + 'static) -> CloudWatchLogsClient {
        let config = aws_sdk_cloudwatchlogs::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new("test", "test", None, None, "test"))
            .timeout_config(TimeoutConfig::disabled())
            .sleep_impl(InstantSleep)
            .http_client(http_client)
            .build();
        CloudWatchLogsClient::from_conf(config)
    }

    #[derive(Debug, Clone)]
    struct DelayedConnector {
        latency: Duration,
        calls: Arc<AtomicU64>,
        status: u16,
    }

    impl DelayedConnector {
        fn new(latency: Duration, status: u16) -> (Self, Arc<AtomicU64>) {
            let calls = Arc::new(AtomicU64::new(0));
            (
                Self {
                    latency,
                    calls: calls.clone(),
                    status,
                },
                calls,
            )
        }
    }

    impl HttpConnector for DelayedConnector {
        fn call(&self, _req: aws_smithy_runtime_api::http::Request) -> HttpConnectorFuture {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let latency = self.latency;
            let status = self.status;
            HttpConnectorFuture::new(async move {
                tokio::time::sleep(latency).await;
                Ok(aws_smithy_runtime_api::http::Response::new(
                    status.try_into().unwrap(),
                    SdkBody::from("{}"),
                ))
            })
        }
    }

    impl HttpClient for DelayedConnector {
        fn http_connector(
            &self,
            _settings: &HttpConnectorSettings,
            _components: &RuntimeComponents,
        ) -> SharedHttpConnector {
            self.clone().into_shared()
        }
    }

    struct TestEntry(u64);

    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("test_value", &self.0);
        }
    }

    #[derive(Debug)]
    struct TestMetrics {
        dropped: AtomicUsize,
        failures: AtomicUsize,
    }

    impl TestMetrics {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                dropped: AtomicUsize::new(0),
                failures: AtomicUsize::new(0),
            })
        }
    }

    impl CwLogsStreamObserver for TestMetrics {
        fn observe(&self, event: &CwLogsStreamEvent) {
            match event {
                CwLogsStreamEvent::BatchDropped { event_count } => {
                    self.dropped.fetch_add(*event_count, Ordering::Relaxed);
                }
                CwLogsStreamEvent::SubmissionFailed => {
                    self.failures.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }

    impl CwLogsStreamObserver for Arc<TestMetrics> {
        fn observe(&self, event: &CwLogsStreamEvent) {
            (**self).observe(event);
        }
    }
    fn default_config() -> CwLogsStreamConfig {
        CwLogsStreamConfig {
            channel_capacity: NonZeroUsize::new(4).unwrap(),
            auto_create: false,
        }
    }

    fn default_stream() -> (CwLogsStream, CwLogsStreamHandle) {
        default_stream_with(test_client_success(), default_config(), None)
    }

    fn default_stream_with(
        client: CloudWatchLogsClient,
        config: CwLogsStreamConfig,
        observer: Option<Box<dyn CwLogsStreamObserver>>,
    ) -> (CwLogsStream, CwLogsStreamHandle) {
        let builder = CwLogsStream::builder()
            .client(client)
            .log_group_name("g".to_string())
            .log_stream_name("s".to_string())
            .namespace("Ns".to_string())
            .config(config);
        #[cfg(not(feature = "tokio_runtime"))]
        let builder = builder.task_spawner(explicit_tokio_test_spawner());
        match observer {
            Some(reporter) => builder.observer(reporter).build(),
            None => builder.build(),
        }
    }

    #[cfg(not(feature = "tokio_runtime"))]
    fn explicit_tokio_test_spawner() -> TaskSpawner {
        TaskSpawner::new(|future| {
            tokio::spawn(future);
        })
    }

    fn futures_thread_spawner() -> TaskSpawner {
        TaskSpawner::new(|future| {
            std::thread::spawn(|| futures::executor::block_on(future));
        })
    }

    #[test]
    fn custom_futures_spawner_runs_outside_tokio_runtime() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = test_client(infallible_client_fn({
            let calls = calls.clone();
            move |_req| {
                calls.fetch_add(1, Ordering::Relaxed);
                success_response()
            }
        }));
        let (mut stream, handle) = CwLogsStream::builder()
            .client(client)
            .log_group_name("g".to_string())
            .log_stream_name("s".to_string())
            .namespace("Ns".to_string())
            .config(default_config())
            .task_spawner(futures_thread_spawner())
            .build();

        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();

        futures::executor::block_on(handle.shutdown());

        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn batch_triggers_send() {
        let (mut stream, handle) = default_stream();

        // Add one entry to measure its serialized size (use value 0 throughout
        // so all entries are identical size).
        stream.next(&TestEntry(0)).unwrap();
        let entry_size = stream.batch_bytes;
        let entries_per_batch = MAX_BATCH_BYTES / entry_size;

        // Reset.
        stream.flush().unwrap();

        // Fill to the batch capacity (doesn't trigger yet due to integer rounding).
        for _ in 0..entries_per_batch {
            stream.next(&TestEntry(0)).unwrap();
        }
        assert_eq!(stream.batch.len(), entries_per_batch);

        // One more entry crosses the byte limit, triggering auto-flush.
        stream.next(&TestEntry(0)).unwrap();
        assert_eq!(stream.batch.len(), 1);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn flush_sends_partial_batch() {
        let (mut stream, handle) = default_stream();

        stream.next(&TestEntry(1)).unwrap();
        assert!(!stream.batch.is_empty());

        stream.flush().unwrap();
        assert!(stream.batch.is_empty());

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn backpressure_drops_and_reports() {
        let metrics = TestMetrics::new();

        // Channel capacity 1, batch size 1 — easy to fill.
        let (mut stream, handle) = default_stream_with(
            test_client_success(),
            CwLogsStreamConfig {
                channel_capacity: NonZeroUsize::new(1).unwrap(),
                auto_create: false,
            },
            Some(Box::new(metrics.clone()) as Box<dyn CwLogsStreamObserver>),
        );

        // On current_thread runtime, the submit loop can't drain the channel
        // until we yield. So the channel fills after the first flush and
        // subsequent flushes are dropped.
        for i in 0..50 {
            stream.next(&TestEntry(i)).unwrap();
            // Flushing after each entry simulates a batch size of 1 for test simplicity.
            stream.flush().unwrap();
        }

        handle.shutdown().await;

        // First flush fills the channel, remaining 49 are dropped.
        assert_eq!(metrics.dropped.load(Ordering::Relaxed), 49);
    }

    #[tokio::test]
    async fn empty_flush_is_noop() {
        let (mut stream, handle) = default_stream();

        stream.flush().unwrap();

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn drop_without_shutdown() {
        // Dropping the stream without calling handle.shutdown() should not panic.
        let (mut stream, _handle) = default_stream();

        stream.next(&TestEntry(1)).unwrap();
        drop(stream);
        // handle is also dropped without shutdown — no panic.
    }

    #[tokio::test]
    #[traced_test]
    async fn drop_without_shutdown_warns_with_pending_entries() {
        let (mut stream, _handle) = default_stream();

        stream.next(&TestEntry(1)).unwrap();
        assert!(stream.submit_task_spawned);
        drop(stream);

        assert!(logs_contain(
            "CwLogsStream dropped without calling shutdown()"
        ));
    }

    #[tokio::test]
    async fn drop_handle_without_shutdown() {
        // Dropping the handle without calling shutdown is fine — stream keeps working
        // until it is dropped.
        let (mut stream, handle) = default_stream();

        drop(handle);

        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        // stream dropped without graceful shutdown — Drop aborts task.
    }

    #[tokio::test]
    async fn shutdown_drains_pending_batches() {
        let (connector, calls) = DelayedConnector::new(Duration::from_millis(50), 200);
        let client = test_client(connector);
        let (mut stream, handle) = default_stream_with(client, default_config(), None);

        // Enqueue 3 batches.
        for batch in 0..3 {
            stream.next(&TestEntry(batch)).unwrap();
            stream.flush().unwrap();
        }

        // Shutdown waits for all 3 to be submitted.
        handle.shutdown().await;
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn submit_loop_reports_failure() {
        let metrics = TestMetrics::new();

        let (mut stream, handle) = default_stream_with(
            test_client_failing(),
            default_config(),
            Some(Box::new(metrics.clone()) as Box<dyn CwLogsStreamObserver>),
        );

        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        handle.shutdown().await;

        assert_eq!(metrics.failures.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    #[traced_test]
    async fn next_after_channel_close_is_noop() {
        let (mut stream, handle) = default_stream_with(
            test_client_success(),
            CwLogsStreamConfig {
                channel_capacity: NonZeroUsize::new(1).unwrap(),
                auto_create: false,
            },
            None,
        );

        // Shutdown closes the channel from the worker side.
        handle.shutdown().await;

        // Next entry after shutdown should be a no-op (channel closed detected on flush).
        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();

        // Subsequent entries take the early-return path (tx is None).
        stream.next(&TestEntry(2)).unwrap();
        assert!(stream.tx.is_none());
        assert!(logs_contain("called after shutdown, entry dropped"));
    }

    #[tokio::test]
    #[traced_test]
    async fn shutdown_after_drop_still_drains() {
        let (connector, calls) = DelayedConnector::new(Duration::from_millis(100), 200);
        let (mut stream, handle) =
            default_stream_with(test_client(connector), default_config(), None);

        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        stream.next(&TestEntry(2)).unwrap();
        stream.flush().unwrap();

        // Yield so the submit loop picks up the first batch (now in-flight for 100ms).
        tokio::task::yield_now().await;

        // Drop the stream while the first batch is still in-flight and the
        // second is queued. This simulates BackgroundQueue dropping the stream.
        assert!(calls.load(Ordering::Relaxed) <= 1);
        drop(stream);

        // Shutdown returns immediately (WeakSender can't upgrade after stream
        // drop), but the worker still drains buffered payloads.
        handle.shutdown().await;

        // Wait for worker to finish.
        for _ in 0..50 {
            if calls.load(Ordering::Relaxed) == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(calls.load(Ordering::Relaxed) == 2);

        assert!(logs_contain(
            "CwLogsStream dropped without calling shutdown()"
        ));
    }

    #[tokio::test]
    async fn shutdown_rejects_entries_enqueued_during_drain() {
        let metrics = TestMetrics::new();
        // Slow connector: each batch takes 50ms, giving us a window to try
        // sending more entries after shutdown begins.
        let (connector, calls) = DelayedConnector::new(Duration::from_millis(50), 200);
        let (mut stream, handle) = default_stream_with(
            test_client(connector),
            default_config(),
            Some(Box::new(metrics.clone()) as Box<dyn CwLogsStreamObserver>),
        );

        // Enqueue a batch before shutdown.
        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();

        // Shutdown closes the receiver — drain processes queued batches but
        // rejects anything new.
        handle.shutdown().await;

        // After shutdown, the channel is closed. Enqueuing should be a no-op.
        stream.next(&TestEntry(2)).unwrap();
        stream.flush().unwrap();

        // Only the pre-shutdown batch was submitted.
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        // Post-shutdown entries are silently discarded, not counted as backpressure drops.
        assert_eq!(metrics.dropped.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn shutdown_drains_entries_queued_while_batch_in_flight() {
        // Slow connector: first batch takes 50ms. While it's in-flight,
        // we enqueue a second batch, then trigger shutdown. Both should drain.
        let (connector, calls) = DelayedConnector::new(Duration::from_millis(50), 200);
        let (mut stream, handle) =
            default_stream_with(test_client(connector), default_config(), None);

        // First batch — starts a 50ms in-flight submission.
        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        // Yield so the submit loop picks up the first batch.
        tokio::task::yield_now().await;

        // Second batch — queued while first is in-flight.
        stream.next(&TestEntry(2)).unwrap();
        stream.flush().unwrap();

        // Shutdown should drain both: the in-flight one finishes, then the
        // queued one is processed during the drain loop.
        handle.shutdown().await;

        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn shutdown_drain_reports_submission_failures() {
        let metrics = TestMetrics::new();
        let (connector, _calls) = DelayedConnector::new(Duration::from_millis(10), 503);
        let client = test_client(connector);
        let (mut stream, handle) = default_stream_with(
            client,
            default_config(),
            Some(Box::new(metrics.clone()) as Box<dyn CwLogsStreamObserver>),
        );

        // Enqueue 2 batches, then shutdown — both fail during drain.
        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        stream.next(&TestEntry(2)).unwrap();
        stream.flush().unwrap();
        handle.shutdown().await;

        assert_eq!(metrics.failures.load(Ordering::Relaxed), 2);
        assert!(logs_contain("CloudWatch Logs PutLogEvents failed"));
    }

    #[tokio::test]
    async fn auto_create_log_resources() {
        let (connector, calls) = DelayedConnector::new(Duration::from_millis(1), 200);
        let client = test_client(connector);
        let (mut stream, handle) = default_stream_with(
            client,
            CwLogsStreamConfig {
                channel_capacity: NonZeroUsize::new(4).unwrap(),
                auto_create: true,
            },
            None,
        );

        // Give the submit loop time to run CreateLogGroup + CreateLogStream.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // At least 2 calls for create_log_group + create_log_stream.
        assert!(calls.load(Ordering::Relaxed) >= 2);

        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        handle.shutdown().await;

        // 2 create calls + 1 PutLogEvents.
        assert!(calls.load(Ordering::Relaxed) == 3);
    }

    #[traced_test]
    #[tokio::test]
    async fn rejected_log_events_are_logged() {
        let config = aws_sdk_cloudwatchlogs::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new("test", "test", None, None, "test"))
            .http_client(infallible_client_fn(|_req| {
                let body = r#"{"rejectedLogEventsInfo":{"tooOldLogEventEndIndex":2}}"#;
                http::Response::builder()
                    .status(200)
                    .body(SdkBody::from(body))
                    .unwrap()
            }))
            .build();
        let client = CloudWatchLogsClient::from_conf(config);

        let (mut stream, handle) = default_stream_with(client, default_config(), None);

        stream.next(&TestEntry(1)).unwrap();
        stream.flush().unwrap();
        handle.shutdown().await;

        assert!(logs_contain("CloudWatch Logs rejected events"));
    }

    #[tokio::test]
    async fn background_queue_drains_after_handle_drop() {
        // 50ms per batch — batches are provably still in-flight when BQ handle drops.
        let (connector, calls) = DelayedConnector::new(Duration::from_millis(50), 200);

        let (stream, _cw_handle) =
            default_stream_with(test_client(connector), default_config(), None);

        let (queue, bg_handle) = BackgroundQueueBuilder::new().build_boxed(stream);

        // Emit entries via BackgroundQueue → CwLogsStream → submit task.
        for i in 0..4u64 {
            queue.append_any(TestEntry(i));
        }

        // Drop queue + BG handle — flushes BQ, drops the stream.
        // CW submit task keeps running in the background.
        drop(queue);
        drop(bg_handle);

        // Not all batches have completed yet — work is still in-flight.
        // Not a race: on current_thread runtime, the submit task can't poll
        // until we yield. Payloads are in the channel but unprocessed.
        assert!(calls.load(Ordering::Relaxed) == 0);

        // Wait for the worker to finish draining.
        // sleep().await yields to the executor so the submit task can progress.
        for _ in 0..50 {
            if calls.load(Ordering::Relaxed) >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(calls.load(Ordering::Relaxed) == 1);
    }
}
