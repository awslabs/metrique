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

/// Whether the target platform implements load average. Matches the set of
/// targets where sysinfo carries a real implementation (anything outside this
/// list returns `0.0` from `System::load_average()`).
const LOAD_AVERAGE_SUPPORTED: bool = cfg!(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "windows",
));

/// Configuration for system metrics bridge subscriptions.
///
/// By default the bridge tracks CPU, memory, swap, load average, uptime, and
/// current-process metrics. Disk, network, and component (thermal) metrics
/// are opt-in via [`Self::with_disks`], [`Self::with_networks`], and
/// [`Self::with_components`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
#[must_use]
pub struct SysinfoMetricsConfig {
    /// Sampling interval used by the reporter loop.
    interval: Duration,
    /// Name style for emitted metric fields.
    name_style: MetricNameStyle,
    /// Whether to include aggregated disk metrics.
    track_disks: bool,
    /// Whether to include aggregated network metrics.
    track_networks: bool,
    /// Whether to include component (thermal sensor) metrics.
    track_components: bool,
}

impl Default for SysinfoMetricsConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_METRIC_SAMPLING_INTERVAL,
            name_style: MetricNameStyle::default(),
            track_disks: false,
            track_networks: false,
            track_components: false,
        }
    }
}

impl SysinfoMetricsConfig {
    /// Return a config with a custom sampling interval.
    ///
    /// Defaults to 30 seconds. Intervals below
    /// [`sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`] (~200ms) produce unreliable
    /// CPU readings, since each sample calls `refresh_cpu_all` and CPU usage
    /// is computed from the delta between refreshes.
    ///
    /// [`sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`]: https://docs.rs/sysinfo/latest/sysinfo/constant.MINIMUM_CPU_UPDATE_INTERVAL.html
    pub fn with_interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }

    /// Set the name style for emitted metric fields.
    ///
    /// Defaults to [`MetricNameStyle::Identity`].
    pub fn with_name_style(self, name_style: MetricNameStyle) -> Self {
        Self { name_style, ..self }
    }

    /// Include aggregated disk metrics in each snapshot. Disabled by default.
    pub fn with_disks(self) -> Self {
        Self {
            track_disks: true,
            ..self
        }
    }

    /// Include aggregated network metrics in each snapshot. Disabled by default.
    pub fn with_networks(self) -> Self {
        Self {
            track_networks: true,
            ..self
        }
    }

    /// Include component (thermal sensor) metrics in each snapshot. Disabled by default.
    pub fn with_components(self) -> Self {
        Self {
            track_components: true,
            ..self
        }
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
#[metrics(subfield_owned)]
#[non_exhaustive]
pub struct SysinfoMetrics {
    // ----- CPU -----
    /// Global CPU usage percentage, averaged across all cores (0.0 to 100.0).
    pub cpu_usage: f32,

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
    //
    // `None` on platforms where sysinfo doesn't implement load average
    // (currently only the `unknown` fallback target — Linux, macOS, iOS,
    // Android, FreeBSD, NetBSD, and Windows all carry real implementations).
    /// 1-minute load average.
    pub load_average_one: Option<f64>,
    /// 5-minute load average.
    pub load_average_five: Option<f64>,
    /// 15-minute load average.
    pub load_average_fifteen: Option<f64>,

    /// System uptime in seconds.
    pub uptime: u64,

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
    /// Bytes read from disk by the current process since the previous refresh.
    pub process_disk_read_bytes: u64,
    /// Cumulative bytes read from disk by the current process since its start.
    pub process_disk_total_read_bytes: u64,
    /// Bytes written to disk by the current process since the previous refresh.
    pub process_disk_written_bytes: u64,
    /// Cumulative bytes written to disk by the current process since its start.
    pub process_disk_total_written_bytes: u64,
    /// Number of open file descriptors held by the current process. `None` on
    /// platforms where sysinfo can't determine it.
    pub process_open_files: Option<usize>,
    /// Soft `RLIMIT_NOFILE` for the current process — the maximum number of
    /// file descriptors it may have open. `None` on platforms where sysinfo
    /// can't determine it.
    pub process_open_files_limit: Option<usize>,

    // ----- Optional categories -----
    /// Aggregated disk metrics. `None` (entirely omitted from output) unless
    /// [`SysinfoMetricsConfig::with_disks`] is set.
    #[metrics(flatten)]
    pub disks: Option<DiskMetrics>,
    /// Aggregated network metrics. `None` (entirely omitted from output) unless
    /// [`SysinfoMetricsConfig::with_networks`] is set.
    #[metrics(flatten)]
    pub networks: Option<NetworkMetrics>,
    /// Component (thermal sensor) metrics. `None` (entirely omitted from
    /// output) unless [`SysinfoMetricsConfig::with_components`] is set.
    #[metrics(flatten)]
    pub components: Option<ComponentMetrics>,
}

/// Aggregated disk metrics across all mounted disks. Emitted as part of
/// [`SysinfoMetrics`] when [`SysinfoMetricsConfig::with_disks`] is set.
#[metrics(subfield_owned)]
#[non_exhaustive]
pub struct DiskMetrics {
    /// Number of mounted disks.
    pub disk_count: u64,
    /// Sum of total space across all mounted disks, in bytes.
    pub total_disk_space: u64,
    /// Sum of available space across all mounted disks, in bytes.
    pub available_disk_space: u64,
}

/// Aggregated network metrics across all interfaces. Emitted as part of
/// [`SysinfoMetrics`] when [`SysinfoMetricsConfig::with_networks`] is set.
#[metrics(subfield_owned)]
#[non_exhaustive]
pub struct NetworkMetrics {
    /// Number of network interfaces being tracked.
    pub network_interface_count: u64,
    /// Comma-separated list of interface names being tracked at sampling
    /// time, e.g. `"eth0,lo,wlan0"`.
    pub network_interfaces: String,
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
}

/// Component (thermal sensor) metrics. Emitted as part of [`SysinfoMetrics`]
/// when [`SysinfoMetricsConfig::with_components`] is set.
#[metrics(subfield_owned)]
#[non_exhaustive]
pub struct ComponentMetrics {
    /// Number of thermal/component sensors being tracked.
    pub component_count: u64,
    /// Maximum current temperature across all components, in degrees Celsius.
    /// `0.0` if no component reports a temperature.
    pub component_max_temperature: f32,
    /// Maximum recorded temperature across all components, in degrees Celsius.
    /// `0.0` if no component reports a max.
    pub component_max_temperature_recorded: f32,
}

/// Extension methods for subscribing system metrics to a global entry sink.
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
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use metrique::emf::Emf;
    /// use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, sink::global_entry_sink};
    /// use metrique_util::{
    ///     AttachGlobalEntrySinkSysinfoExt, MetricNameStyle, SysinfoMetricsConfig,
    /// };
    /// use std::time::Duration;
    ///
    /// global_entry_sink! { ServiceMetrics }
    ///
    /// let emf = Emf::builder("MyService".to_string(), vec![vec![]])
    ///     .build()
    ///     .output_to_makewriter(|| std::io::stderr().lock());
    /// let _handle = ServiceMetrics::attach_to_stream(emf);
    ///
    /// let config = SysinfoMetricsConfig::default()
    ///     .with_interval(Duration::from_secs(30))
    ///     .with_name_style(MetricNameStyle::PascalCase);
    /// ServiceMetrics::subscribe_sysinfo_metrics(config);
    /// ```
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
    disks: Option<&mut Disks>,
    networks: Option<&mut Networks>,
    components: Option<&mut Components>,
    pid: Option<sysinfo::Pid>,
) -> SysinfoMetrics {
    system.refresh_memory();
    system.refresh_cpu_all();
    if let Some(pid) = pid {
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    }

    let process = pid.and_then(|p| system.process(p));
    let load = System::load_average();

    let disks_metrics = disks.map(|d| {
        d.refresh(true);
        let (total, available) = d.list().iter().fold((0u64, 0u64), |(t, a), disk| {
            (t + disk.total_space(), a + disk.available_space())
        });
        DiskMetrics {
            disk_count: d.list().len() as u64,
            total_disk_space: total,
            available_disk_space: available,
        }
    });

    let networks_metrics = networks.map(|n| {
        n.refresh(true);
        let interfaces = n.list().keys().cloned().collect::<Vec<_>>().join(",");
        let mut agg = NetworkMetrics {
            network_interface_count: n.list().len() as u64,
            network_interfaces: interfaces,
            network_received: 0,
            network_total_received: 0,
            network_transmitted: 0,
            network_total_transmitted: 0,
            network_packets_received: 0,
            network_total_packets_received: 0,
            network_packets_transmitted: 0,
            network_total_packets_transmitted: 0,
            network_errors_on_received: 0,
            network_total_errors_on_received: 0,
            network_errors_on_transmitted: 0,
            network_total_errors_on_transmitted: 0,
        };
        for data in n.list().values() {
            agg.network_received += data.received();
            agg.network_total_received += data.total_received();
            agg.network_transmitted += data.transmitted();
            agg.network_total_transmitted += data.total_transmitted();
            agg.network_packets_received += data.packets_received();
            agg.network_total_packets_received += data.total_packets_received();
            agg.network_packets_transmitted += data.packets_transmitted();
            agg.network_total_packets_transmitted += data.total_packets_transmitted();
            agg.network_errors_on_received += data.errors_on_received();
            agg.network_total_errors_on_received += data.total_errors_on_received();
            agg.network_errors_on_transmitted += data.errors_on_transmitted();
            agg.network_total_errors_on_transmitted += data.total_errors_on_transmitted();
        }
        agg
    });

    let components_metrics = components.map(|c| {
        c.refresh(true);
        let (max_cur, max_recorded) = c.list().iter().fold((0.0f32, 0.0f32), |(cur, max), comp| {
            (
                cur.max(comp.temperature().unwrap_or(0.0)),
                max.max(comp.max().unwrap_or(0.0)),
            )
        });
        ComponentMetrics {
            component_count: c.list().len() as u64,
            component_max_temperature: max_cur,
            component_max_temperature_recorded: max_recorded,
        }
    });

    SysinfoMetrics {
        cpu_usage: system.global_cpu_usage(),

        total_memory: system.total_memory(),
        used_memory: system.used_memory(),
        available_memory: system.available_memory(),
        free_memory: system.free_memory(),
        total_swap: system.total_swap(),
        used_swap: system.used_swap(),
        free_swap: system.free_swap(),

        load_average_one: LOAD_AVERAGE_SUPPORTED.then_some(load.one),
        load_average_five: LOAD_AVERAGE_SUPPORTED.then_some(load.five),
        load_average_fifteen: LOAD_AVERAGE_SUPPORTED.then_some(load.fifteen),

        uptime: System::uptime(),

        process_memory: process.map(|p| p.memory()).unwrap_or(0),
        process_virtual_memory: process.map(|p| p.virtual_memory()).unwrap_or(0),
        process_cpu_usage: process.map(|p| p.cpu_usage()).unwrap_or(0.0),
        process_accumulated_cpu_time: process.map(|p| p.accumulated_cpu_time()).unwrap_or(0),
        process_run_time: process.map(|p| p.run_time()).unwrap_or(0),
        process_disk_read_bytes: process.map(|p| p.disk_usage().read_bytes).unwrap_or(0),
        process_disk_total_read_bytes: process
            .map(|p| p.disk_usage().total_read_bytes)
            .unwrap_or(0),
        process_disk_written_bytes: process.map(|p| p.disk_usage().written_bytes).unwrap_or(0),
        process_disk_total_written_bytes: process
            .map(|p| p.disk_usage().total_written_bytes)
            .unwrap_or(0),
        process_open_files: process.and_then(|p| p.open_files()),
        process_open_files_limit: process.and_then(|p| p.open_files_limit()),

        disks: disks_metrics,
        networks: networks_metrics,
        components: components_metrics,
    }
}

fn spawn_sysinfo_metrics_task(
    sink: BoxEntrySink,
    config: SysinfoMetricsConfig,
) -> tokio::task::AbortHandle {
    let interval = config.interval;
    let name_style = config.name_style;
    let track_disks = config.track_disks;
    let track_networks = config.track_networks;
    let track_components = config.track_components;
    let worker = tokio::spawn(async move {
        tracing::debug!("sysinfo metrics reporter started");
        let mut system = System::new();
        let mut disks = track_disks.then(Disks::new_with_refreshed_list);
        let mut networks = track_networks.then(Networks::new_with_refreshed_list);
        let mut components = track_components.then(Components::new_with_refreshed_list);
        let pid = sysinfo::get_current_pid().ok();

        // Prime delta-based readings (CPU usage, network rx/tx since last
        // refresh, etc.) so the first emitted sample has accurate values.
        // sysinfo computes those from the time delta between two refreshes.
        system.refresh_cpu_all();
        if let Some(pid) = pid {
            system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        }
        if let Some(n) = networks.as_mut() {
            n.refresh(true);
        }
        tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;

        loop {
            let snapshot = sample(
                &mut system,
                disks.as_mut(),
                networks.as_mut(),
                components.as_mut(),
                pid,
            );
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
    }

    #[tokio::test(start_paused = true)]
    async fn opt_in_categories_emit_their_fields() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_disks()
                .with_networks()
                .with_components(),
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entries = inspector.entries();
        check!(!entries.is_empty());
        let entry = entries.last().unwrap();
        check!(entry.metrics["disk_count"] > 0);
        check!(entry.metrics["total_disk_space"] > 0);
        check!(entry.metrics["network_interface_count"] >= 0);
        check!(entry.metrics["component_count"] >= 0);
    }

    #[tokio::test(start_paused = true)]
    async fn process_level_metrics_are_populated() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entry = inspector.entries().last().cloned().unwrap();

        // The reporter resolves the current PID, so each process_* key should
        // be present (indexing panics when a key is missing, so a `>= 0`
        // probe is enough to prove Option<T> resolved to Some).
        check!(entry.metrics["process_memory"] > 0);
        check!(entry.metrics["process_virtual_memory"] > 0);
        check!(entry.metrics["process_cpu_usage"] >= 0.0);
        check!(entry.metrics["process_accumulated_cpu_time"] >= 0);
        check!(entry.metrics["process_run_time"] >= 0);
        check!(entry.metrics["process_disk_read_bytes"] >= 0);
        check!(entry.metrics["process_disk_total_read_bytes"] >= 0);
        check!(entry.metrics["process_disk_written_bytes"] >= 0);
        check!(entry.metrics["process_disk_total_written_bytes"] >= 0);
        // stdin / stdout / stderr alone account for at least 3 open FDs.
        check!(entry.metrics["process_open_files"] > 0);
        check!(entry.metrics["process_open_files_limit"] > 0);
    }

    #[tokio::test(start_paused = true)]
    async fn only_disks_enabled() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_disks(),
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entry = inspector.entries().last().cloned().unwrap();
        check!(entry.metrics.keys().any(|k| k.starts_with("disk_")));
    }

    #[tokio::test(start_paused = true)]
    async fn only_networks_enabled() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_networks(),
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entry = inspector.entries().last().cloned().unwrap();
        check!(entry.metrics.keys().any(|k| k.starts_with("network_")));
    }

    #[tokio::test(start_paused = true)]
    async fn only_components_enabled() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default()
                .with_interval(Duration::from_millis(50))
                .with_components(),
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entry = inspector.entries().last().cloned().unwrap();
        check!(entry.metrics.keys().any(|k| k.starts_with("component_")));
    }

    #[tokio::test(start_paused = true)]
    async fn opt_in_categories_omitted_by_default() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _handle = Sink::attach((sink, ()));

        Sink::subscribe_sysinfo_metrics(
            SysinfoMetricsConfig::default().with_interval(Duration::from_millis(50)),
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let entry = inspector.entries().last().cloned().unwrap();
        let keys: Vec<&String> = entry.metrics.keys().collect();
        check!(!keys.iter().any(|k| k.starts_with("disk_")));
        check!(!keys.iter().any(|k| k.starts_with("network_")));
        check!(!keys.iter().any(|k| k.starts_with("component_")));
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
