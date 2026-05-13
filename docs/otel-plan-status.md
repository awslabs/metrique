# `metrique-otel` —

current status and plan

Follow-up to the [original design comment](https://github.com/awslabs/metrique/issues/110#issuecomment-4400001687)
and both [Jess's](https://github.com/awslabs/metrique/issues/110#issuecomment-4422783595) and [Russell's](https://github.com/awslabs/metrique/issues/110#issuecomment-4412942585) feedback.
Captures where the `otel-integration` branch landed after [#282 (Entry Descriptors)](https://github.com/awslabs/metrique/pull/282),
what's intentionally deferred, and what changes once [#289 (field shape lowering)](https://github.com/awslabs/metrique/pull/289)
ships.

## What's implemented

### End user API

```rust
use std::time::Duration;
use metrique::{ServiceMetrics, timers::Timer, unit_of_work::metrics};
use metrique::writer::AttachGlobalEntrySink;
use metrique_otel::OtelSink;
use metrique_otel::tags::{Counter, UpDownCounter, Histogram, Gauge};

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct RequestMetrics {
    time: Timer,
    operation: String,                                                  // metric attribute on every observation
    #[metrics(field_tag(Counter))]       request_count: u64,            // OTel Counter::add
    #[metrics(field_tag(UpDownCounter))] queue_depth:   i64,            // OTel UpDownCounter::add
    #[metrics(field_tag(Histogram), unit = Millisecond)] latency: Duration, // OTel Histogram::record
    #[metrics(field_tag(Gauge))]         cpu_usage:     f64,            // OTel Gauge::record
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

The translator runs in two stages:

1. **Plan build (once per descriptor shape).** `OtelSink` owns an `EntryPlan` cache. On first sight of a shape it walks `entry.descriptors()`, resolves each field's `tags()` to an `InstrumentKind`, derives the `InstrumentationScope` as `metrique/{desc.name()}`, and stores the plan keyed by a composite hash of descriptor segment ids. Misconfigured numeric fields (a numeric field with no recognised instrument-kind tag) trigger a single `tracing::warn!` per shape, never per write, in `OtelSink::plan_for` at `metrique-otel/src/lib.rs`. Different entry types produce different OTel `InstrumentationScope`s.

2. **Hot path (per entry).** The translator consults the cached plan via one read-lock per append and classifies fields through a `HashMap<String, FieldKind>`. For each closed `Entry` it produces:
   - a bag of **entry-wide attributes** built from every string field encountered during the walk. These attributes get attached to every metric observation produced by the same entry, so saying `operation = GetBook` labels every counter, histogram and gauge from that request;
   - a list of **pending observations**, one per numeric field whose tag resolved to an instrument kind (`Counter` / `UpDownCounter` / `Histogram` / `Gauge`).

**Aggregation fallback.** `metrique-aggregation`'s `Distribution` `MetricFlags` is still recognised in the translator as a runtime fallback. This is what keeps the `#[aggregate(strategy = Histogram<T>)]` path working without anybody having to tag the underlying value.

**Dispatch.** At `finish()`, each pending observation is dispatched against an `InstrumentCache` that lazily creates the right OTel instrument (`u64_counter`, `i64_up_down_counter`, `f64_histogram`, `f64_gauge`) by `(name, kind)` and feeds it the observation plus the merged attribute set (entry-wide + per-metric dimensions). Units round-trip via a UCUM mapping (`ms`, `us`, `By`, `KBy`, etc).

### Built on the full OTel SDK

The current implementation depends on `opentelemetry`, `opentelemetry_sdk`, and
`opentelemetry-otlp` since it's the low hanging fruit to prototype the solution. The option of using the
`opentelemetry-proto` + direct tonic exporter is valid and pretty interesting. Some analysis is available at[`docs/minimal-exporter-comparison.md`](../../code/wye/metrique/docs/minimal-exporter-comparison.md).
I would keep working on the full OTel SDK until we have a clear idea what's the best approach for the long term and when we fine, start working on the minimal path, which makes total sense for long term.

## Landed with #282 (Yet in progress)

The pieces that were blocked on Entry Descriptors are now in:

- **Instrument-kind selection via field tags.** `#[metrics(field_tag(Counter))]`
  (and friends) replaced the `ForceFlag` wrapper API (`Counter<T>` / `UpDownCounter<T>` / `Histogram<T>` / `Gauge<T>`). The zero-sized tag markers live at `metrique_otel::tags::*`. The `AddAssign` / `SubAssign` impls that had been added to `ForceFlag` to keep `Counter<u64>` usable inside `Sum` strategies were reverted, that need disappears once the type is plain `u64`.
- **Per-`DescriptorId` `EntryPlan` cache** replacing the per-entry walk. The hot path is now a fixed traversal over the cached plan instead of re-inferring kinds per call. This is also a precondition for the minimal exporter path: the encoder needs to know shape/kind/unit once per descriptor, not per observation.
- **Silent-drop surfaced** as a one-shot `tracing::warn!` per descriptor shape, listing the offending fields and the instrumentation scope and hinting at the correct attribute. No more silent drops at runtime.
- **`InstrumentationScope` from `desc.name()`.** `Meter::name` is now derived per entry shape (`metrique {desc.name()}`), so different entry types produce different OTel scopes that the exporter sees as separate `InstrumentationScope`s.

## Still TBD / Questions

- **Field shape lowering (#289).** Every `FieldView::shape()` returns `FieldShape::Opaque` today. Without scalar-distribution shape info we cannot preclassify "this field will emit a scalar or a histogram" at plan-build time, so we still rely on the `Distribution` runtime flag for the aggregation path. Once #289 ships, the plan can be richer and the runtime fallback can shrink.
- **`FieldShape::Distribution` variant.** Not in #289 yet. Same impact as above, preserves the `Distribution` flag fallback.
- **Custom entry names.** `EntryDescriptor::name()` exists but there is no `entry_name` attribute override yet. Scope name is always the Rust struct name; users who want a different OTel scope name must wait.
- **`default_field_tag` ergonomics for OTel.** I considered adding an `Emit` tag with `default_field_tag(Emit)` + `skip(Emit)` to let users explicitly opt fields out of OTel emission. Deferred to a later iteration, the current heuristic (string > attribute, kind-tagged > observation) covers the common case.
- **Setup**: took the easy path which is configuring OTel's collector by env
  vars as the SDK proposes, we probably want differently or in a standard
  way.
- **Delta vs cumulative temporality.** Descriptors describe shape, notaggregation policy. I think that a builder option (likely`with_temporality(..)`) and a story for cumulative state alongside`metrique-aggregation` is the way. This is pretty much the reason to keepthe SDK for a first iteration: cumulative is free there, and we'd have toreimplement the accumulator on the minimal path.
- **Decide when to swap to the minimal exporter path** (see
  [`docs/minimal-exporter-comparison.md`](./minimal-exporter-comparison.md)).
