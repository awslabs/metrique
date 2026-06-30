An [OpenTelemetry (OTLP)][otlp] backend for [`metrique`][metrique].

`metrique-otel` bridges metrique's aggregation pipeline to the OpenTelemetry
SDK: declare a metrics struct, pick an aggregation strategy, tag the OTel
instrument kind on each field, and export merged observations over OTLP
(gRPC or HTTP).

Aggregation happens on a worker thread before reaching the OTel SDK, so the
SDK only sees one merged observation per key tuple per flush, substantially
cheaper per ingest than recording on every entry. The same struct can fan out
to non-OTel sinks (e.g. EMF), which ignore the instrument-kind tags.

For more detail, read the crate docs for [`OtelSink`][OtelSink].

## Quick start

```rust
use std::time::Duration;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::value::Sum;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_otel::OtelSink;
use metrique_otel::flags::Counter;

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[aggregate(key)] operation: String,

    #[aggregate(strategy = Sum)]
    #[metrics(flags(Counter))]
    request_count: u64,

    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let otel_sink = OtelSink::with_otlp_default()?;
let aggregator = KeyedAggregator::<RequestMetrics, _>::new(otel_sink.clone());
let worker = WorkerSink::new(aggregator, Duration::from_secs(1));

RequestMetrics {
    operation: "GET".into(),
    request_count: 1,
    latency: Duration::from_millis(12),
}
.close_and_merge(worker.clone());

worker.flush().await;
otel_sink.flush_async().await;
# Ok(()) }
```

`with_otlp_default` uses OTLP/gRPC (tonic) and must run inside a Tokio runtime.
For a non-async context, [`OtelSink::with_otlp_http_default`][http] uses a
blocking HTTP transport that needs no runtime. Both read the standard `OTEL_*`
environment variables (e.g. `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`),
defaulting to `localhost:4317` for gRPC and `localhost:4318` for HTTP. To control
the OTel `Resource`, views, or temporality, build your own `SdkMeterProvider` and
pass it via `OtelSink::builder().with_meter_provider(...)` (see the
`otlp_resource_attributes` and `otlp_views_and_temporality` examples).
`OtelSink::builder().with_scope(...)` sets the OTel `InstrumentationScope` name
(default `"metrique-otel"`), handy when one app drives multiple sinks. Built
against `opentelemetry` 0.32.

## Instrument kinds

Counters, up-down counters, and gauges are selected by tagging the field with
[`flags(Counter)`][flags], `flags(UpDownCounter)`, or `flags(Gauge)`. Histograms
need no tag: a `Histogram` strategy advertises a distribution that the
translator maps to a histogram instrument. Non-OTel sinks ignore the flags.

## Behavior

- Per-entry timestamps are dropped; OTel readers stamp measurements with their
  own clock.
- Every non-metric `String` field on the entry becomes an attribute on all
  metrics in that same entry, even ones declared before it (records are buffered
  until the entry finishes). This is how dimensions ride along.
- `#[metrics(unit = ...)]` maps to the UCUM string OTel expects (`ms`, `By`, `%`,
  `1` for dimensionless). The unit is fixed when the instrument is first created;
  later differing units on the same name are ignored.
- A `Histogram` `Repeated` observation (a pre-summed batch) is replayed as its
  mean, capped at 1024 replays. Bucketing is lossy (percentiles pinch toward the
  mean) and occurrences beyond the cap undercount. For faithful distributions,
  avoid pre-summing histogram fields (e.g. don't put a `Sum`-style strategy on
  them).
- Out-of-range observations (NaN, or negative on a counter) are dropped with a
  rate-limited `warn`.
- A non-histogram field with no `flags(...)` tag is dropped, not given a default
  instrument, so forgetting the tag surfaces as a missing metric rather than a
  silent miscount.
- Conflicting instrument kinds wrapping one value resolve first-wins (the
  innermost kind survives); debug builds panic on the conflict.
- The sink never shuts down or flushes the meter provider on drop. Call `flush` /
  `flush_async` (or shut the provider down yourself) before exit, or the final
  export window is lost.

## Examples

See the `examples/` directory for complete, runnable wiring:

- `otlp_aggregated` - canonical aggregation pipeline
- `otlp_alongside_emf` - dual emission to OTLP and EMF
- `otlp_multi_entry` - multiple entry types
- `otlp_resource_attributes` - custom resource attributes
- `otlp_views_and_temporality` - views and temporality config

Run with: `cargo run --example <name>`

[otlp]: https://opentelemetry.io/docs/specs/otlp/
[metrique]: https://docs.rs/metrique
[OtelSink]: https://docs.rs/metrique-otel/latest/metrique_otel/struct.OtelSink.html
[http]: https://docs.rs/metrique-otel/latest/metrique_otel/struct.OtelSink.html#method.with_otlp_http_default
[flags]: https://docs.rs/metrique-otel/latest/metrique_otel/flags/index.html
[KeyedAggregator]: https://docs.rs/metrique-aggregation/latest/metrique_aggregation/aggregator/struct.KeyedAggregator.html
