// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use crate::dynamic_inflection::DynamicInflectionEntry;
use metrique::CloseValue;
use metrique::unit_of_work::metrics;
use metrique::writer::{AttachGlobalEntrySink, BoxEntrySink, EntrySink, ShutdownFn};
use metrique_core::DynamicNameStyle as MetricNameStyle;
use sysinfo::{Components, Disks, Networks, ProcessesToUpdate, System};

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
/// `process_*` fields measure the process running this reporter: i.e. your
/// service binary. Child processes, forked workers, and sidecars are not
/// included.
///
/// [sysinfo docs]: https://docs.rs/sysinfo
#[metrics(subfield)]
pub struct SysinfoMetrics {
    // ----- CPU -----
    /// Global CPU usage percentage, averaged across all cores (0.0 to 100.0).
    pub cpu_usage: f32,
    /// Number of logical CPUs visible to the process.
    pub num_cpus: u64,
    /// Number of physical cores. `0` if sysinfo can't determine it on the host.
    pub physical_core_count: u64,

    // ----- Memory -----
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

    // ----- Load average -----
    /// 1-minute load average. `0.0` on platforms without load average support.
    pub load_average_one: f64,
    /// 5-minute load average. `0.0` on platforms without load average support.
    pub load_average_five: f64,
    /// 15-minute load average. `0.0` on platforms without load average support.
    pub load_average_fifteen: f64,

    /// System uptime in seconds.
    pub uptime: u64,

    // ----- Disks (aggregated across all mounted disks) -----
    /// Number of mounted disks.
    pub disk_count: u64,
    /// Sum of total space across all mounted disks, in bytes.
    pub total_disk_space: u64,
    /// Sum of available space across all mounted disks, in bytes.
    pub available_disk_space: u64,

    // ----- Networks (aggregated across all interfaces) -----
    /// Number of network interfaces being tracked.
    pub network_interface_count: u64,
    /// Bytes received across all interfaces since the previous refresh.
    pub network_received: u64,
    /// Cumulative bytes received across all interfaces since interface tracking began.
    pub network_total_received: u64,
    /// Bytes transmitted across all interfaces since the previous refresh.
    pub network_transmitted: u64,
    /// Cumulative bytes transmitted across all interfaces since interface tracking began.
    pub network_total_transmitted: u64,
    /// Packets received across all interfaces since the previous refresh.
    pub network_packets_received: u64,
    /// Cumulative packets received across all interfaces.
    pub network_total_packets_received: u64,
    /// Packets transmitted across all interfaces since the previous refresh.
    pub network_packets_transmitted: u64,
    /// Cumulative packets transmitted across all interfaces.
    pub network_total_packets_transmitted: u64,
    /// Receive errors across all interfaces since the previous refresh.
    pub network_errors_on_received: u64,
    /// Cumulative receive errors across all interfaces.
    pub network_total_errors_on_received: u64,
    /// Transmit errors across all interfaces since the previous refresh.
    pub network_errors_on_transmitted: u64,
    /// Cumulative transmit errors across all interfaces.
    pub network_total_errors_on_transmitted: u64,

    // ----- Components (thermal sensors) -----
    /// Number of thermal/component sensors being tracked.
    pub component_count: u64,
    /// Maximum current temperature across all components, in degrees Celsius.
    /// `0.0` if no component reports a temperature.
    pub component_max_temperature: f32,
    /// Maximum recorded temperature across all components, in degrees Celsius.
    /// `0.0` if no component reports a max.
    pub component_max_temperature_recorded: f32,

    // ----- Current process -----
    /// Resident memory in bytes for the current process.
    pub process_memory: u64,
    /// Virtual memory in bytes for the current process.
    pub process_virtual_memory: u64,
    /// CPU usage percentage for the current process. May exceed 100% on
    /// multi-core systems.
    pub process_cpu_usage: f32,
    /// Total CPU time accumulated by the current process, in milliseconds.
    pub process_accumulated_cpu_time: u64,
    /// Time the current process has been running, in seconds.
    pub process_run_time: u64,
    /// Wall-clock time when the current process started, as a Unix timestamp.
    pub process_start_time: u64,
    /// Bytes read from disk by the current process since the previous refresh.
    pub process_disk_read_bytes: u64,
    /// Cumulative bytes read from disk by the current process since its start.
    pub process_disk_total_read_bytes: u64,
    /// Bytes written to disk by the current process since the previous refresh.
    pub process_disk_written_bytes: u64,
    /// Cumulative bytes written to disk by the current process since its start.
    pub process_disk_total_written_bytes: u64,
    /// Number of open file descriptors held by the current process. `0` on
    /// platforms where sysinfo can't determine it.
    pub process_open_files: u64,
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

fn sample(
    system: &mut System,
    disks: &mut Disks,
    networks: &mut Networks,
    components: &mut Components,
    pid: Option<sysinfo::Pid>,
) -> SysinfoMetrics {
    system.refresh_memory();
    system.refresh_cpu_all();
    if let Some(pid) = pid {
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    }
    disks.refresh(true);
    networks.refresh(true);
    components.refresh(true);

    let process = pid.and_then(|p| system.process(p));
    let load = System::load_average();

    let (total_disk_space, available_disk_space) =
        disks.list().iter().fold((0u64, 0u64), |(t, a), d| {
            (t + d.total_space(), a + d.available_space())
        });

    let mut net = NetworkAggregate::default();
    for data in networks.list().values() {
        net.received += data.received();
        net.total_received += data.total_received();
        net.transmitted += data.transmitted();
        net.total_transmitted += data.total_transmitted();
        net.packets_received += data.packets_received();
        net.total_packets_received += data.total_packets_received();
        net.packets_transmitted += data.packets_transmitted();
        net.total_packets_transmitted += data.total_packets_transmitted();
        net.errors_on_received += data.errors_on_received();
        net.total_errors_on_received += data.total_errors_on_received();
        net.errors_on_transmitted += data.errors_on_transmitted();
        net.total_errors_on_transmitted += data.total_errors_on_transmitted();
    }

    let (component_max_temperature, component_max_temperature_recorded) = components
        .list()
        .iter()
        .fold((0.0f32, 0.0f32), |(cur, max), c| {
            (
                cur.max(c.temperature().unwrap_or(0.0)),
                max.max(c.max().unwrap_or(0.0)),
            )
        });

    SysinfoMetrics {
        cpu_usage: system.global_cpu_usage(),
        num_cpus: system.cpus().len() as u64,
        physical_core_count: System::physical_core_count().unwrap_or(0) as u64,

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

        disk_count: disks.list().len() as u64,
        total_disk_space,
        available_disk_space,

        network_interface_count: networks.list().len() as u64,
        network_received: net.received,
        network_total_received: net.total_received,
        network_transmitted: net.transmitted,
        network_total_transmitted: net.total_transmitted,
        network_packets_received: net.packets_received,
        network_total_packets_received: net.total_packets_received,
        network_packets_transmitted: net.packets_transmitted,
        network_total_packets_transmitted: net.total_packets_transmitted,
        network_errors_on_received: net.errors_on_received,
        network_total_errors_on_received: net.total_errors_on_received,
        network_errors_on_transmitted: net.errors_on_transmitted,
        network_total_errors_on_transmitted: net.total_errors_on_transmitted,

        component_count: components.list().len() as u64,
        component_max_temperature,
        component_max_temperature_recorded,

        process_memory: process.map(|p| p.memory()).unwrap_or(0),
        process_virtual_memory: process.map(|p| p.virtual_memory()).unwrap_or(0),
        process_cpu_usage: process.map(|p| p.cpu_usage()).unwrap_or(0.0),
        process_accumulated_cpu_time: process.map(|p| p.accumulated_cpu_time()).unwrap_or(0),
        process_run_time: process.map(|p| p.run_time()).unwrap_or(0),
        process_start_time: process.map(|p| p.start_time()).unwrap_or(0),
        process_disk_read_bytes: process.map(|p| p.disk_usage().read_bytes).unwrap_or(0),
        process_disk_total_read_bytes: process
            .map(|p| p.disk_usage().total_read_bytes)
            .unwrap_or(0),
        process_disk_written_bytes: process.map(|p| p.disk_usage().written_bytes).unwrap_or(0),
        process_disk_total_written_bytes: process
            .map(|p| p.disk_usage().total_written_bytes)
            .unwrap_or(0),
        process_open_files: process
            .and_then(|p| p.open_files())
            .map(|v| v as u64)
            .unwrap_or(0),
    }
}

#[derive(Default)]
struct NetworkAggregate {
    received: u64,
    total_received: u64,
    transmitted: u64,
    total_transmitted: u64,
    packets_received: u64,
    total_packets_received: u64,
    packets_transmitted: u64,
    total_packets_transmitted: u64,
    errors_on_received: u64,
    total_errors_on_received: u64,
    errors_on_transmitted: u64,
    total_errors_on_transmitted: u64,
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
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let mut components = Components::new_with_refreshed_list();
        let pid = sysinfo::get_current_pid().ok();

        // Prime delta-based readings (CPU usage, network rx/tx since last
        // refresh, etc.) so the first emitted sample has accurate values.
        // sysinfo computes those from the time delta between two refreshes.
        system.refresh_cpu_all();
        if let Some(pid) = pid {
            system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        }
        networks.refresh(true);
        tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;

        loop {
            let snapshot = sample(&mut system, &mut disks, &mut networks, &mut components, pid);
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

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["total_memory"] > 0);
        check!(entry.metrics["uptime"] > 0);
        check!(entry.metrics["num_cpus"] > 0);
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

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["TotalMemory"] > 0);
        check!(entry.metrics["Uptime"] > 0);
        check!(entry.metrics["NumCpus"] > 0);
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

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["total_memory"] > 0);
        check!(entry.metrics["uptime"] > 0);
        check!(entry.metrics["num_cpus"] > 0);
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

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());

        let entry = entries.last().unwrap();
        check!(entry.metrics["total-memory"] > 0);
        check!(entry.metrics["uptime"] > 0);
        check!(entry.metrics["num-cpus"] > 0);
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
        tokio::time::sleep(Duration::from_millis(500)).await;
        let count_before = inspector.entries().len();
        check!(count_before > 0);

        // Dropping the handle should abort the reporter task.
        drop(handle);

        // Advance time further, no new entries should be appended.
        tokio::time::sleep(Duration::from_millis(500)).await;
        check!(inspector.entries().len() == count_before);
    }
}
