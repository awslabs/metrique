// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Emitting metrics from a periodic background task — a unit of work that is not a request.
//!
//! A cache refresher, a queue poller, a heartbeat: each tick is its own unit of
//! work, so each tick gets its own entry with its own timestamp, duration and
//! fault count. The metric struct is written exactly as in
//! `./unit-of-work-simple.rs`; what differs is who drives it and when it stops.
//!
//! This is the right shape when the *tick* is the thing you want to measure. If
//! instead you want the state a background task samples — CPU, queue depth,
//! cache size — to ride along on every request record, reach for the other
//! flavor the reporters offer: [`embed_sysinfo_metrics`] hands back a `State`
//! you fold into your own entries with `#[metrics(flatten)]`, and no standalone
//! entry is emitted at all. The cookbook's "Periodic metrics" section covers
//! that trade-off.
//!
//! The wiring follows the same shape as the `metrique-util` reporters
//! ([`subscribe_sysinfo_metrics`] and `subscribe_tokio_runtime_metrics`): an
//! extension trait on the global entry sink that
//!
//! 1. takes the sink with [`BoxEntrySink::lazy`], so the loop may be started
//!    before — or without — a sink ever being attached (entries are dropped
//!    until one is),
//! 2. spawns the loop, and
//! 3. hands the sink a [`ShutdownFn`] so the loop stops when the `AttachHandle`
//!    is dropped instead of outliving the metric stream.
//!
//! Two things the real reporters do are worth copying, and one is a place
//! where this example deliberately differs:
//!
//! - **Cancel between ticks, not during one.** The shutdown signal is only
//!   awaited while waiting for the next tick, so a refresh already in flight
//!   runs to completion and emits its entry. Aborting the task instead would
//!   drop the future mid-tick, and since the metric is an append-on-drop
//!   guard, the half-finished tick would append an entry reporting zero work
//!   and a truncated duration.
//! - **Survive a panic.** The reporters wrap the whole loop body in
//!   [`std::panic::catch_unwind`] and log the payload, so a panicking sample
//!   doesn't silently end the metric stream. Omitted here to keep the example
//!   short.
//! - **Don't assume a runtime.** [`tokio::spawn`] panics outside one, which is
//!   fine here because the work is async. The reporters sample with blocking
//!   syscalls, so they use [`tokio::task::spawn_blocking`] when a runtime is
//!   present and fall back to [`std::thread::spawn`] when it isn't.
//!
//! [`embed_sysinfo_metrics`]: https://docs.rs/metrique-util/latest/metrique_util/trait.AttachGlobalEntrySinkSysinfoExt.html#method.embed_sysinfo_metrics
//! [`subscribe_sysinfo_metrics`]: https://docs.rs/metrique-util/latest/metrique_util/trait.AttachGlobalEntrySinkSysinfoExt.html#method.subscribe_sysinfo_metrics

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use metrique::ServiceMetrics;
use metrique::emf::Emf;
use metrique::timers::{Timer, Timestamp};
use metrique::unit::Count;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySink, AttachGlobalEntrySinkExt, BoxEntrySink, FormatExt, ShutdownFn,
};
use tokio::sync::oneshot;
use tokio::time::MissedTickBehavior;

const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Knobs for the refresh loop, shaped like the reporters' own configs
/// ([`SysinfoMetricsConfig`], `TokioRuntimeMetricsConfig`): `#[non_exhaustive]`
/// with consuming `with_*` setters, so a new knob can be added later without
/// breaking callers that already build one.
///
/// [`SysinfoMetricsConfig`]: https://docs.rs/metrique-util/latest/metrique_util/struct.SysinfoMetricsConfig.html
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
#[must_use]
pub struct CacheRefreshMetricsConfig {
    /// How often a refresh runs.
    interval: Duration,
    /// Value of the `Task` dimension on every emitted entry.
    task: &'static str,
}

impl Default for CacheRefreshMetricsConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_REFRESH_INTERVAL,
            task: "cache-refresh",
        }
    }
}

impl CacheRefreshMetricsConfig {
    /// Run a refresh every `interval`. Defaults to 30 seconds.
    pub fn with_interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }

    /// Label entries from this loop, so it can be told apart from any other
    /// background work writing to the same sink. Defaults to `cache-refresh`.
    pub fn with_task(self, task: &'static str) -> Self {
        Self { task, ..self }
    }
}

/// One entry per refresh. `task` tells this loop apart from any other
/// background work writing to the same sink.
#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct CacheRefreshMetrics {
    // Set when the tick starts. Without this the entry is stamped when it is
    // flushed, which for a slow tick can land in the next minute.
    #[metrics(timestamp)]
    timestamp: Timestamp,

    task: &'static str,

    // Runs from entry creation to entry close: the duration of one tick, not
    // of the loop.
    time: Timer,

    #[metrics(unit = Count)]
    entries_refreshed: usize,

    #[metrics(unit = Count)]
    failures: usize,
}

impl CacheRefreshMetrics {
    fn init(task: &'static str, sink: BoxEntrySink) -> CacheRefreshMetricsGuard {
        CacheRefreshMetrics {
            task,
            ..Default::default()
        }
        .append_on_drop(sink)
    }
}

/// Plugs the refresh loop into a global entry sink, the way the `metrique-util`
/// reporters plug themselves in. Blanket-implemented, so it works for
/// [`ServiceMetrics`] and for any sink declared with `global_entry_sink!`.
trait AttachGlobalEntrySinkCacheRefreshExt: AttachGlobalEntrySink + 'static {
    /// Spawn the refresh loop. Must be called from within a Tokio runtime.
    ///
    /// The loop stops at the next tick boundary once the sink's `AttachHandle`
    /// is dropped; it runs forever if that handle is `forget`ten.
    fn subscribe_cache_refresh_metrics(config: CacheRefreshMetricsConfig) {
        let sink = BoxEntrySink::lazy(Self::try_sink);
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(config.interval);
            // A slow refresh should delay the next tick, not make the loop
            // fire back-to-back trying to catch up.
            ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                // Shutdown is awaited here and nowhere else: a refresh that has
                // already started always finishes and emits its entry.
                tokio::select! {
                    _ = ticks.tick() => {}
                    _ = &mut cancel_rx => return,
                }
                refresh_once(&sink, config.task).await;
            }
        });
        // Dropping the sender wakes the receiver, the same way the
        // `metrique-util` reporters signal their worker.
        Self::register_shutdown_fn(ShutdownFn::new(move || drop(cancel_tx)));
    }
}

impl<T: AttachGlobalEntrySink + 'static> AttachGlobalEntrySinkCacheRefreshExt for T {}

/// A single unit of work: the guard is created at the top of the tick and
/// appended to the sink when it drops at the bottom of it.
async fn refresh_once(sink: &BoxEntrySink, task: &'static str) {
    let mut metrics = CacheRefreshMetrics::init(task, sink.clone());
    match refresh_cache().await {
        Ok(refreshed) => metrics.entries_refreshed = refreshed,
        Err(err) => {
            // A failed tick still emits an entry — that is what makes the
            // failure alarmable.
            tracing::warn!("cache refresh failed: {err}");
            metrics.failures = 1;
        }
    }
}

/// Stand-in for the real work, failing every third tick so the example emits
/// both outcomes.
async fn refresh_cache() -> Result<usize, &'static str> {
    static TICK: AtomicUsize = AtomicUsize::new(0);
    tokio::time::sleep(Duration::from_millis(10)).await;
    match TICK.fetch_add(1, Ordering::Relaxed) {
        tick if tick % 3 == 2 => Err("upstream timed out"),
        _ => Ok(7),
    }
}

#[tokio::main]
async fn main() {
    // Faster than the default so the example finishes quickly.
    const REFRESH_INTERVAL: Duration = Duration::from_millis(100);

    tracing_subscriber::fmt::init();

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("BackgroundTaskExample".to_string(), vec![vec![]])
            .output_to_makewriter(std::io::stdout),
    );

    ServiceMetrics::subscribe_cache_refresh_metrics(
        CacheRefreshMetricsConfig::default()
            .with_interval(REFRESH_INTERVAL)
            .with_task("cache-refresh"),
    );

    // In a service the loop lives as long as the attach handle does. Here three
    // ticks run — the wait ends mid-interval, clear of the fourth tick — then
    // `_handle` drops and the `ShutdownFn` stops the loop.
    tokio::time::sleep(REFRESH_INTERVAL * 2 + REFRESH_INTERVAL / 2).await;

    // EXAMPLE OUTPUT
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"BackgroundTaskExample","Dimensions":[[]],"Metrics":[{"Name":"Time","Unit":"Milliseconds"},{"Name":"EntriesRefreshed","Unit":"Count"},{"Name":"Failures","Unit":"Count"}]}],"Timestamp":1789584359629},"Time":11.197667,"EntriesRefreshed":7,"Failures":0,"Task":"cache-refresh"}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"BackgroundTaskExample","Dimensions":[[]],"Metrics":[{"Name":"Time","Unit":"Milliseconds"},{"Name":"EntriesRefreshed","Unit":"Count"},{"Name":"Failures","Unit":"Count"}]}],"Timestamp":1789584359729},"Time":11.238475,"EntriesRefreshed":7,"Failures":0,"Task":"cache-refresh"}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"BackgroundTaskExample","Dimensions":[[]],"Metrics":[{"Name":"Time","Unit":"Milliseconds"},{"Name":"EntriesRefreshed","Unit":"Count"},{"Name":"Failures","Unit":"Count"}]}],"Timestamp":1789584359829},"Time":11.349112,"EntriesRefreshed":0,"Failures":1,"Task":"cache-refresh"}
    */
}
