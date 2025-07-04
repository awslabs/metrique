## Best Practices for when using `metrique` in Libraries

For applications using metrics (e.g. you are authoring a `main.rs` somewhere) this document is not for you!

This document covers best practices when author a library that you produces metrics. See `examples/library-provided-metric.rs` for a fully worked example.

### Use `Instrumented` to wrap return values

`metrique` offers the `Instrumented` struct to provide a consistent interface for libraries to expose metrics along with their return types. For more information, see the `instrument` module and `library-provided-metric.rs` example.

### Avoid using `#[metrics(rename_all = "...")]`

If you use `metrics(rename_all)`, then callers won't be able to apply a consistent naming scheme to your metrics.

### Use `#[metrics(subfield)]` to avoid generating unecessary code.
By default, `#[metrics]` generates a full metrics implementation that can be flushed to a sink. However, this isn't necessary for libraries where your entry will typically be included as part of a larger entry.

### Do not expose private fields from your metrics
To maintain API stability, **do not** make the fields of your metric public unless you are prepared to uphold that guarantee. Instead, you should have snapshot tests of the format of your metrics and the keys/values that are emitted.
