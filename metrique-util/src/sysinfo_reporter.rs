// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use crate::dynamic_inflection::DynamicInflectionEntry;
use metrique::CloseValue;
use metrique::unit_of_work::metrics;
use metrique::writer::{AttachGlobalEntrySink, BoxEntrySink, EntrySink, ShutdownFn};
use metrique_core::DynamicNameStyle as MetricNameStyle;
use sysinfo::{ProcessesToUpdate, System};

const DEFAULT_METRIC_SAMPLING_INTERVAL: Duration = Duration::from_secs(30);

/// Configuration for system metrics bridge subscriptions.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
#[must_use]
pub struct SysinfoMetricsConfig {
    /// Sampling interval used by the reporter loop.
    interval: Duration,
    /// Name style for emitted metric fields.
    name_style: MetricNameStyle,
}

impl Default for SysinfoMetricsConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_METRIC_SAMPLING_INTERVAL,
            name_style: MetricNameStyle::default(),
        }
    }
}

impl SysinfoMetricsConfig {
    /// Return a config with a custom sampling interval.
    pub fn with_interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }

    /// Set the name style for emitted metric fields.
    ///
    /// Defaults to [`MetricNameStyle::Identity`].
    pub fn with_name_style(self, name_style: MetricNameStyle) -> Self {
        Self { name_style, ..self }
    }
}

/// A snapshot of system-wide and current-process metrics sampled from
/// [`sysinfo`].
///
/// Field names mirror sysinfo's API verbatim. See the [sysinfo docs] for
/// platform-specific behavior and the meaning of each value.
///
/// [sysinfo docs]: https://docs.rs/sysinfo
#[metrics]
pub struct SysinfoMetrics {
    /// Global CPU usage percentage, averaged across all cores (0.0 to 100.0).
    pub cpu_usage: f32,
    /// Total physical memory in bytes.
    pub total_memory: u64,
    /// Used physical memory in bytes.
    pub used_memory: u64,
    /// Available memory in bytes (free + reclaimable from caches).
    pub available_memory: u64,
    /// Free memory in bytes (truly unused).
    pub free_memory: u64,
    /// Total swap space in bytes.
    pub total_swap: u64,
    /// Used swap space in bytes.
    pub used_swap: u64,
    /// Free swap space in bytes.
    pub free_swap: u64,
    /// 1-minute load average. `0.0` on platforms without load average support.
    pub load_average_one: f64,
    /// 5-minute load average. `0.0` on platforms without load average support.
    pub load_average_five: f64,
    /// 15-minute load average. `0.0` on platforms without load average support.
    pub load_average_fifteen: f64,
    /// System uptime in seconds.
    pub uptime: u64,
    /// Resident memory in bytes for the current process.
    pub process_memory: u64,
    /// Virtual memory in bytes for the current process.
    pub process_virtual_memory: u64,
    /// CPU usage percentage for the current process. May exceed 100% on
    /// multi-core systems.
    pub process_cpu_usage: f32,
    /// Cumulative bytes read from disk by the current process since its start.
    pub process_disk_total_read_bytes: u64,
    /// Cumulative bytes written to disk by the current process since its start.
    pub process_disk_total_written_bytes: u64,
}

/// Extension methods for subscribing system metrics to a global entry sink.
///
/// Spawns a background task that periodically samples [`SysinfoMetrics`] from
/// [`sysinfo`] and appends each snapshot to the sink. The task is automatically
/// aborted when the [`AttachHandle`](metrique::writer::sink::AttachHandle) is
/// dropped.
///
/// # Example
///
/// ```rust,ignore
/// use metrique_util::{
///     AttachGlobalEntrySinkSysinfoExt, MetricNameStyle, SysinfoMetricsConfig,
/// };
/// use std::time::Duration;
///
/// let _handle = ServiceMetrics::attach_to_stream(emf.output_to(std::io::stderr()));
///
/// let config = SysinfoMetricsConfig::default()
///     .with_interval(Duration::from_secs(30))
///     .with_name_style(MetricNameStyle::PascalCase);
/// ServiceMetrics::subscribe_sysinfo_metrics(config);
/// ```
pub trait AttachGlobalEntrySinkSysinfoExt: AttachGlobalEntrySink + 'static {
    /// Subscribe to system metrics, adding the subscription to this handle.
    ///
    /// Spawns a background task that periodically samples [`SysinfoMetrics`]
    /// and appends each snapshot to the sink.
    ///
    /// The reporter task is automatically aborted when the
    /// [`AttachHandle`](metrique::writer::sink::AttachHandle) is dropped. If
    /// the handle is [`forgotten`](metrique::writer::sink::AttachHandle::forget),
    /// the reporter runs indefinitely.
    ///
    /// If no sink has been attached yet, entries are silently discarded until
    /// one is attached.
    fn subscribe_sysinfo_metrics(config: SysinfoMetricsConfig) {
        let sink = BoxEntrySink::lazy(Self::try_sink);
        let abort = spawn_sysinfo_metrics_task(sink, config);
        Self::register_shutdown_fn(ShutdownFn::new(move || {
            abort.abort();
        }));
    }
}

impl<T: AttachGlobalEntrySink + 'static> AttachGlobalEntrySinkSysinfoExt for T {}

fn sample(system: &mut System, pid: Option<sysinfo::Pid>) -> SysinfoMetrics {
    system.refresh_memory();
    system.refresh_cpu_all();
    if let Some(pid) = pid {
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    }

    let process = pid.and_then(|p| system.process(p));
    let load = System::load_average();

    SysinfoMetrics {
        cpu_usage: system.global_cpu_usage(),
        total_memory: system.total_memory(),
        used_memory: system.used_memory(),
        available_memory: system.available_memory(),
        free_memory: system.free_memory(),
        total_swap: system.total_swap(),
        used_swap: system.used_swap(),
        free_swap: system.free_swap(),
        load_average_one: load.one,
        load_average_five: load.five,
        load_average_fifteen: load.fifteen,
        uptime: System::uptime(),
        process_memory: process.map(|p| p.memory()).unwrap_or(0),
        process_virtual_memory: process.map(|p| p.virtual_memory()).unwrap_or(0),
        process_cpu_usage: process.map(|p| p.cpu_usage()).unwrap_or(0.0),
        process_disk_total_read_bytes: process
            .map(|p| p.disk_usage().total_read_bytes)
            .unwrap_or(0),
        process_disk_total_written_bytes: process
            .map(|p| p.disk_usage().total_written_bytes)
            .unwrap_or(0),
    }
}

fn spawn_sysinfo_metrics_task(
    sink: BoxEntrySink,
    config: SysinfoMetricsConfig,
) -> tokio::task::AbortHandle {
    let interval = config.interval;
    let name_style = config.name_style;
    let worker = tokio::spawn(async move {
        tracing::debug!("sysinfo metrics reporter started");
        let mut system = System::new();
        let pid = sysinfo::get_current_pid().ok();
        loop {
            let snapshot = sample(&mut system, pid);
            sink.append(DynamicInflectionEntry {
                entry: snapshot.close(),
                name_style,
            });
            tokio::time::sleep(interval).await;
        }
    });
    let abort = worker.abort_handle();

    // Spawn a monitor to log panics
    tokio::spawn(async move {
        if let Err(err) = worker.await
            && !err.is_cancelled()
        {
            tracing::error!("sysinfo metrics reporter panicked: {err}");
        }
    });
    abort
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use assert2::check;
    use metrique_writer::sink::AttachGlobalEntrySink;
    use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

    use super::{AttachGlobalEntrySinkSysinfoExt, MetricNameStyle, SysinfoMetricsConfig};

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_identity() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["total_memory"].as_u64() > 0);
        check!(entry.metrics.contains_key("cpu_usage"));
        check!(entry.metrics.contains_key("uptime"));
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_pascal_case() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_name_style(MetricNameStyle::PascalCase),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["TotalMemory"].as_u64() > 0);
        check!(entry.metrics.contains_key("CpuUsage"));
        check!(entry.metrics.contains_key("Uptime"));
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_snake_case() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_name_style(MetricNameStyle::SnakeCase),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["total_memory"].as_u64() > 0);
        check!(entry.metrics.contains_key("cpu_usage"));
        check!(entry.metrics.contains_key("uptime"));
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_appends_metrics_kebab_case() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_name_style(MetricNameStyle::KebabCase),
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["total-memory"].as_u64() > 0);
        check!(entry.metrics.contains_key("cpu-usage"));
        check!(entry.metrics.contains_key("uptime"));
    }

    #[tokio::test(start_paused = true)]
    async fn subscribe_aborted_on_handle_drop() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        // Let some entries accumulate.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let count_before = inspector.entries().len();
        check!(count_before > 0);

        // Dropping the handle should abort the reporter task.
        drop(handle);

        // Advance time further, no new entries should be appended.
        tokio::time::sleep(Duration::from_millis(200)).await;
        check!(inspector.entries().len() == count_before);
    }
}
