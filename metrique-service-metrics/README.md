This package defines a `ServiceMetrics` [global entry sink], that can be used by different parts of an application to share metrics.

This is a separate crate just to allow different major versions of `metrique` to share a global metrics sink. Most users should use it via the path `metrique::ServiceMetrics` instead of `metrique_service_metrics::ServiceMetrics`.

[global entry sink]: https://docs.rs/metrique-writer/latest/metrique_writer/sink/macro.global_entry_sink.html
