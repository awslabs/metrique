#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use metrique_writer::sink::global_entry_sink;

global_entry_sink! {
    /// A global metric sink that can be used for application-wide
    /// metrics.
    ///
    /// This sink is not treated differently by `metrique` than
    /// any other [`global_entry_sink`], but it easily allows
    /// different parts of an application to stream metrics to
    /// a common destination.
    ///
    /// For use, an application can attach an [EntrySink] to
    /// this global sink, and then write metrics.
    ///
    /// [EntrySink]: metrique_writer::EntrySink
    ///
    /// ## Example
    ///
    /// ```rust
    /// use std::path::PathBuf;
    ///
    /// use metrique::unit_of_work::metrics;
    /// use metrique::ServiceMetrics;
    /// use metrique_writer::GlobalEntrySink;
    /// use metrique_writer::{AttachGlobalEntrySinkExt, FormatExt, sink::AttachHandle};
    /// use metrique_writer_format_emf::Emf;
    /// use tracing_appender::rolling::{RollingFileAppender, Rotation};
    ///
    /// #[metrics(rename_all = "PascalCase")]
    /// struct RequestMetrics {
    ///     number_of_ducks: usize,
    /// }
    ///
    /// impl RequestMetrics {
    ///     fn init() -> RequestMetricsGuard {
    ///         RequestMetrics {
    ///             number_of_ducks: 0,
    ///         }.append_on_drop(ServiceMetrics::sink())
    ///     }
    /// }
    ///
    /// fn initialize_metrics(service_log_dir: PathBuf) -> AttachHandle {
    ///     ServiceMetrics::attach_to_stream(
    ///         Emf::builder("Ns".to_string(), vec![vec![]])
    ///             .build()
    ///             .output_to_makewriter(RollingFileAppender::new(
    ///                 Rotation::MINUTELY,
    ///                 &service_log_dir,
    ///                 "service_log.log",
    ///             )),
    ///     )
    /// }
    ///
    /// let _join = initialize_metrics("my/metrics/dir".into());
    /// let mut metrics = RequestMetrics::init();
    /// metrics.number_of_ducks = 5;
    /// ```
    ServiceMetrics
}
