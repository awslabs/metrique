# `metrique-otel` — current status and plan

Follow-up to the [original design comment](https://github.com/awslabs/metrique/issues/110#issuecomment-4400001687)
and both [Jess's](https://github.com/awslabs/metrique/issues/110#issuecomment-4422783595) and [Russell's](https://github.com/awslabs/metrique/issues/110#issuecomment-4412942585) feedback.
Captures where the `otel-integration` branch landed, what's intentionally deferred,
and what changes once [#282 (Entry Descriptors)](https://github.com/awslabs/metrique/pull/282)
ships.

## What's implemented

### End user API

```rust
use std::time::Duration;
use metrique::{ServiceMetrics, timers::Timer, unit_of_work::metrics};
use metrique::writer::AttachGlobalEntrySink;
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, UpDownCounter, Histogram, Gauge};

#[metrics]
#[derive(Default)]
struct RequestMetrics {
    time: Timer,
    operation: String,                  // metric attribute on every observation
    request_count: Counter<u64>,        // OTel Counter::add
    queue_depth:   UpDownCounter<i64>,  // OTel UpDownCounter::add
    latency_ms:    Histogram<Duration>, // OTel Histogram::record
    cpu_usage:     Gauge<f64>,          // OTel Gauge::record
}

#[tokio::main]
async fn main() {
    // Simple path: OTLP gRPC providers built from OTEL_* env vars.
    let _h = ServiceMetrics::attach_to_sink(OtelSink::with_otlp_default().unwrap());
    handle_request().await;
}
```

A custom builder path (`OtelSink::builder().with_meter_provider(..).with_resource(..).build()`)
and a `flush_async()` for clean shutdown are also exposed. Logs are not emitted, for the time being, since is not a priority to users. Planning to address this in the future.

### Topologies

**Direct path**: every closed `Entry` becomes one OTel observation per metric
field on its own attribute set. This is the low hanging fruit one, suitable for low/medium volume.

```
Entry > OtelSink (EntrySink<E>) > OTLP exporter
```

**Aggregation-fed path**: `metrique-aggregation`
rolls many entries into one bundle per attribute group per flush, and the sink
emits one observation per group:

```
Entry > KeyedAggregator (Sum / Histogram strategies) --flushes--> WorkerSink > OtelSink > OTLP exporter
```

This approach comes from both feedbacks and the concern that metadata next to metrics is not a good idea, so that `#[aggregate(key)]` fields become OTel data-point attributes, and `Sum` / `Distribution` strategies map directly to OTel `Counter` / `Histogram` observations on the right attribute set.

### Translator behavior

For each closed `Entry`, the translator walks it once and produces:

1. A bag of **entry-wide attributes** built from every string field encountered
   during the walk. These attributes get attached to every metric observation
   produced by the same entry, so saying "operation = GetBook" labels every counter,
   histogram and gauge from that request.
2. A list of **pending observations**, one per numeric field carrying an
   instrument-kind hint. The kind is selected from:
   - an explicit aggregation-hint flag (`Counter<T>` / `UpDownCounter<T>` /
     `Histogram<T>` / `Gauge<T>`), implemented today as `ForceFlag`-wrapped
     values that round-trip through metrique's existing `MetricFlags` mechanism;
     or
   - the `Distribution` flag emitted by `metrique-aggregation`'s histogram
     strategy.

   Fields with no kind hint are dropped silently, intended for now, worth to discuss if we want to surface it as a warning/error or tackle down now.

At `finish()`, each pending observation is dispatched against an
`InstrumentCache` that lazily creates the right OTel instrument (`u64_counter`,
`i64_up_down_counter`, `f64_histogram`, `f64_gauge`) by `(name, kind)` and feeds
it the observation plus the merged attribute set (entry-wide + per-metric
dimensions). Units round-trip via a UCUM mapping (`ms`, `us`, `By`, `KBy`, …).

### Built on the full OTel SDK

The current implementation depends on `opentelemetry`, `opentelemetry_sdk`, and
`opentelemetry-otlp` since it's the low hanging fruit to prototype the solution. The option of using the
`opentelemetry-proto` + direct tonic exporter is valid and pretty interesting. Some analysis is available at[`docs/minimal-exporter-comparison.md`](../../code/wye/metrique/docs/minimal-exporter-comparison.md).
I would keep working on the full OTel SDK until we have a clear idea what's the best approach for the long term and when we fine, start working on the minimal path, which makes total sense for long term.

## Pending on #282 (Entry Descriptors)

Once it lands the OTel sink can take the following start using the static tag system and avoid the flags (current approach).

- **Instrument-kind selection moves from `ForceFlag` wrappers to field tags.**
  Users would `#[metrics(field_tag(Counter))]` or struct-level
  `default_field_tag(..)` instead of typing `Counter<u64>`. The
  `AddAssign` / `SubAssign` impls added to `ForceFlag` on this branch become
  unnecessary and can be reverted.
- **Per-entry-type plan instead of per-entry walk.** With
  `EntryDescriptor::name()`, `fields()`, units, and resolved tags available
  ahead of time, the sink builds one plan per `DescriptorId` describing which
  fields are attributes, which are observations, what kind, what unit. The hot
  path becomes a fixed traversal over that plan instead of inferring per call.
  This is also a precondition for the minimal exporter path: the encoder needs
  to know shape/kind/unit once per descriptor, not per observation.
- **Eliminates the silent-drop class of bugs.** Misconfigured numeric fields are
  caught at descriptor inspection (or first emission), not dropped at runtime.
- **Clean attribute/observation separation.** `default_field_tag(skip(Emit))`
  and per-field tags would give users and the sink explicit control over which fields
  are dimensions vs measurements, instead of inferring "string > attribute" by
  walking values.
- **Stable entry name for OTel scope/instrument-name composition.**
  `EntryDescriptor::name()` provides a canonical identifier. Nowadays the sink
  relies on per-field naming.

## Still TBD / Questions

- **Setup**: took the easy path which is configuring OTel's collector by env vars as the SDK proposes, we probably want differently or in a standard way.
- **Delta vs cumulative temporality.** Descriptors describe shape, not
  aggregation policy. I think that a builder option (likely `with_temporality(..)`)
  and a story for cumulative state alongside `metrique-aggregation` is the way. This is pretty much the reason to keep the SDK for a first iteration: cumulative is free there, and we'd
  have to reimplement the accumulator on the minimal path.
- **Surface the silent-drop case** as a `tracing::warn!` + a counter on
  `OtelSink`, so misuse is visible even before #282 lands.
- **Decide when to swap to the minimal exporter path** (see the comparison
  doc).
