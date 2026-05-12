# Full OTel SDK vs. direct `opentelemetry-proto` + tonic exporter

> Status: investigation. No implementation yet. Captures the trade-off so we can decide whether to swap before a first ships.
>
> Parent plan and current status: [`otel-plan-status.md`](./otel-plan-status.md).

## Why look at this

`metrique-otel` today's base implemention, depends on the full OpenTelemetry stack: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` (and through it, `reqwest`, `tonic`, `prost`, the entire SDK metric pipeline). The SDK models metrics as one-off observations recorded against named, cached instruments. Metrique already has the opposite shape: a closed `Entry` is a self-contained measurement bundle with attributes attached, and `metrique-aggregation` can roll many of those into one bundle per attribute set per flush. Passing aggregated bundles through the SDK means re-disaggregating them into per-instrument `add()` / `record()` calls so the SDK can re-aggregate them on the way out,an internal double-aggregation that exists only because the SDK's record API insists on it.

The alternative is to encode `ResourceMetrics` directly from `metrique-aggregation` output and push it over tonic to an OTLP collector, skipping the SDK's record/aggregator layer entirely.

## Dependencies measurement

```
crates in transitive closure (cargo tree --no-dedupe, normal edges)

metrique-otel (today)                          129
    via opentelemetry-otlp                     109
        via opentelemetry-proto                 75
            via tonic                           56
            via prost                           10 (subset of tonic's)
        via reqwest                             88 (largely non-overlapping)
        via opentelemetry_sdk                   38
```

Reading this:

- A "minimal" path using `opentelemetry-proto` + `tonic` directly won't shrink the wire-encoding side. Tonic, prost, http, http-body, etc. are roughly the same crates either way.
- The real savings come from dropping `opentelemetry_sdk` (futures-executor, rand, thiserror, percent-encoding, tokio-stream pulled for sync primitives we don't use) and dropping `reqwest` (pulled transitively by `opentelemetry-otlp` via `opentelemetry-http`, which is an unconditional dependency regardless of the `grpc-tonic` feature verification by runnning: `cargo tree --invert reqwest` against `opentelemetry-otlp`).
- Realistic estimate: a `metrique-otel` that talked to OTLP via `opentelemetry-proto` + `tonic` only would land in the _75–80 crate_ range, down from 129. **Roughly a third less to compile**.
- Transport is its own axis. `tonic` is the obvious choice (gRPC, streaming), but `reqwest` over OTLP/HTTP-protobuf is a smaller-dep alternative that drops more crates than tonic does at the cost of bidirectional streaming. Not a recommendation, just an axis to note.

## What would be loose:

These are features `opentelemetry_sdk` we currently inherit for free:

- **[`PeriodicReader`](https://docs.rs/opentelemetry_sdk/0.32.0/opentelemetry_sdk/metrics/struct.PeriodicReader.html)**: timer-driven export loop with collect > export sequencing, including shutdown handshake.
- **[`SdkMeterProvider`](https://docs.rs/opentelemetry_sdk/0.32.0/opentelemetry_sdk/metrics/struct.SdkMeterProvider.html)**: meter caching, resource attachment, registered views.
- **Instrument lifecycle**: counter/histogram/up-down/gauge SDK types with their own internal aggregation — see the [`opentelemetry::metrics`](https://docs.rs/opentelemetry/0.32.0/opentelemetry/metrics/index.html) module.
- **Resource detection**: env-driven `OTEL_RESOURCE_ATTRIBUTES`, host detection, process attributes. See [`opentelemetry_sdk::resource`](https://docs.rs/opentelemetry_sdk/0.32.0/opentelemetry_sdk/resource/index.html) and the [`Resource`](https://docs.rs/opentelemetry_sdk/0.32.0/opentelemetry_sdk/struct.Resource.html) struct; spec reference: [OTel Resource SDK](https://opentelemetry.io/docs/specs/otel/resource/sdk/).
- **Retry/backoff** on the export path (via [`opentelemetry-otlp`'s `RetryPolicy`](https://docs.rs/opentelemetry-otlp/0.32.0/opentelemetry_otlp/retry/struct.RetryPolicy.html), exposed through [`WithTonicConfig`](https://docs.rs/opentelemetry-otlp/0.32.0/opentelemetry_otlp/trait.WithTonicConfig.html); gated behind the `experimental-grpc-retry` feature).
- **Batch processor** semantics (for logs, we don't have em in this first version), see [`opentelemetry_sdk::logs::BatchLogProcessor`](https://docs.rs/opentelemetry_sdk/0.32.0/opentelemetry_sdk/logs/struct.BatchLogProcessor.html).
- **Aggregation temporality selector** (Delta vs. Cumulative; spec: [OTel data model — Temporality](https://opentelemetry.io/docs/specs/otel/metrics/data-model/#temporality), see the temporality discussion in `otel-plan-status.md`, this is the strongest argument for keeping the SDK if we want cumulative without re-implementing the accumulator ourselves). Cost here is design effort and correctness risk, not code volume: getting cumulative right under restarts, gaps, and attribute-set churn is the hard part, not the storage.

## What we'd reimplement

- Build an `ExportLoop`: a single tokio task that ticks on an interval, pulls the current aggregator state via a `flush()` call, encodes it to `ResourceMetrics`, opens a tonic stream, sends, handles errors.
- Encode strategy → proto:
  - `Sum` → `Metric.data = Sum { is_monotonic, aggregation_temporality, data_points: [NumberDataPoint] }`
  - `Distribution` (histogram) → `Metric.data = Histogram { aggregation_temporality, data_points: [HistogramDataPoint] }` with bucket counts and sum/count, or `ExponentialHistogram` if we keep an exponential bucketing strategy in aggregation.
  - `KeepLast` → `Metric.data = Gauge { data_points: [NumberDataPoint] }`
  - Each aggregation key tuple becomes `attributes: [KeyValue]` on the data point. This is the one place metrique's "metadata next to metrics" lines up cleanly with the wire format, avoids impedance mismatch.
- Resource attachment: read `OTEL_RESOURCE_ATTRIBUTES` (TBD if env var or other way) once at startup, emit it on every `ResourceMetrics`. Host/process detection is _not_ reimplemented so users who need it either stay on the SDK path or set the env var themselves. Calling this out explicitly because the "what we'd lose" list above mentions host/process, this path accepts that loss rather than pulling a detector crate.
- `InstrumentationScope`: emit `ScopeMetrics.scope = { name: "metrique", version: CARGO_PKG_VERSION }`. Trivial, but worth naming since the SDK does it for us today.
- Retry: tonic gives us per-RPC errors. A simple "retry with backoff up to N seconds, then drop" loop should be enough, assuming a sidecar / local collector.
- Cumulative state (if we want it): a side table keyed by attribute set, persisting the cumulative value across flushes.
