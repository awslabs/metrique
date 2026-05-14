# Entry descriptors and field flags

> **Status: partially implemented.** Field flags, descriptors, and flatten chaining are implemented. Field shapes are deferred (all Opaque).

A small system on top of metrique's existing `Entry` / `Value` / `CloseValue` traits that lets sinks introspect the structure of macro-derived entries.

Two pieces, both opt-in for sinks:

- An **entry descriptor** that describes a macro-derived entry's closed shape: ordered fields, their flags, optionality, lists, dynamic-key maps, units, canonical entry name, and an optional timestamp field.
- A **field flag** system that lets sinks define their own static opt-in markers and lets users apply them at struct or field scope.

This adds `descriptors()` to the `Entry` and `InflectableEntry` traits (with defaulted impls). `Value` and `CloseValue` are unchanged. Sinks that do not call `Entry::descriptors()` pay nothing.

## Glossary

- **Entry descriptor** (`EntryDescriptor`): a metrique-emitted description of a macro-derived entry's closed shape. Sinks read it to learn what fields the entry can emit, in what order, with what flags and units, and what the entry is canonically called.
- **Field flag**: a `FlagConstructor` type (e.g. `emf::flags::HighStorageResolution`, `dial9::flags::Export`) that a format or sink crate declares and that users apply to fields via `#[metrics(flags(T))]`. Flags appear in the descriptor for sink introspection and are applied at write time for format behavior.
- **`default_flags` / `flags`**: struct-level and field-level attributes for applying flags. `skip(T)` suppresses a default. `default_flags` applies to direct fields only and does not propagate to flattened children.
- **`FieldShape`**: the closed/emitted shape of a field (scalar, optional, list, dynamic-key map, or opaque). Describes what the sink will see, not the raw Rust type.
- **`DescriptorRef`**: the handle yielded by `Entry::descriptors()`. Provides field access via `FieldView`, carries a stable `DescriptorId` for cache keying.
- **`DescriptorId`**: an opaque identifier for a descriptor, stable within a single process lifetime. Used by sinks to cache derived data.

### Relationship between flags, FlagConstructor, ForceFlag, and MetricFlags

A **`FlagConstructor`** is a type whose `construct()` method returns a `MetricFlags<'static>` value. Format crates (like EMF) define these to represent format-specific options (e.g. `HighStorageResolutionCtor`). They also provide a `flags` module re-exporting the constructor under a user-friendly name (e.g. `emf::flags::HighStorageResolution`).

**`#[metrics(flags(T))]`** where `T: FlagConstructor` does two things:
1. Records `T`'s `TypeId` in the field's descriptor (so sinks can introspect it via `FieldView::flags()`)
2. Wraps the field's value in `ForceFlag<_, T>` at write time (so the flag flows through `ValueWriter::metric()` to the format)

**`ForceFlag<V, T>`** is the write-path mechanism. It intercepts `Value::write` and injects `T::construct()` into the `MetricFlags` parameter of `ValueWriter::metric()`. Using `ForceFlag<V, T>` as a field type directly only affects the write path; the descriptor is unaware of it. Prefer `#[metrics(flags(T))]` to get both.

**`MetricFlags`** is the runtime payload that formats receive in `ValueWriter::metric()`. Formats downcast it to check for specific options (e.g. `flags.downcast::<EmfOptions>()`).

## What it enables

- Sinks can inspect the complete set of fields an entry can emit, including optional fields and dynamic maps, without observing multiple live emissions.
- Sinks and formats can declare per-field opt-in via flags users apply to their entries without sink-specific newtypes on field values.
- First-class units in the descriptor, surfaced however each sink prefers.
- All of the above after `BoxEntry` erasure.

Sinks that do not call `Entry::descriptors()` pay nothing at runtime. The statics are generated unconditionally but only occupy binary size.

## At a glance

Here is the end-to-end shape from a user's perspective. The example uses a made-up `audit` sink to keep the mechanics generic.

```rust
// --- in the `audit` sink crate ---

// The audit sink defines a FlagConstructor. Users apply it via flags(...)
// to opt them into the audit stream.
pub struct Export;

// --- in the application ---

use audit::Export;

// The struct declares `Export` as the default for all its fields; individual
// fields can override with `flags(skip(Export))`.
#[metrics(default_flags(Export))]
struct RequestMetrics {
    // `request_id` inherits the struct default: tagged Export.
    request_id: String,
    // `operation` also inherits the struct default.
    operation: &'static str,
    // `debug_blob` opts out: not in the audit payload.
    #[metrics(flags(skip(Export)))]
    debug_blob: String,
}
```

The macro generates (in addition to the existing `Entry` impl) an `EntryDescriptor` describing three fields: `request_id` (flag: Export, shape: String), `operation` (flag: Export, shape: String), `debug_blob` (no flags, shape: String). The descriptor's canonical name is `"RequestMetrics"`.

An audit sink reads the descriptor at first-use per entry type, walks `Entry::write` in descriptor order, and for each value consults the flags on the descriptor to decide whether to emit that field to its wire format.

## The descriptor model

The descriptor types live in `metrique_writer_core::descriptor`. `Entry::descriptors()` returns a `Descriptors` enum (`Available` or `Unavailable`), forcing callers to handle entries that haven't implemented descriptor support. When available, sinks interact with `DescriptorRef` and `FieldView`. For custom `Entry` implementations, the module provides const builder methods (`EntryDescriptor::builder()`, `FieldDescriptor::builder()`, `TimestampDescriptor::new()`) for constructing descriptors in statics. See the module rustdoc for the full API.

```rust
use metrique_writer_core::descriptor::*;
use metrique_writer_core::{Unit, Descriptors};

static FIELDS: [FieldDescriptor; 2] = [
    FieldDescriptor::builder("latency_ms").unit(Unit::Milliseconds).build(),
    FieldDescriptor::builder("count").unit(Unit::Count).build(),
];
static DESC: EntryDescriptor = EntryDescriptor::builder("MyMetrics", &FIELDS)
    .timestamp(TimestampDescriptor::new("timestamp"))
    .build();

impl Entry for MyMetrics {
    fn descriptors(&self) -> Descriptors<'_> {
        Descriptors::available(std::iter::once(DescriptorRef::from_static(&DESC)))
    }
    // ...
}
```

### Forward compatibility

All public enums in the descriptor module are `#[non_exhaustive]`. Consumers must include a wildcard arm when matching; new variants are additive in minor versions.

All public structs have private fields. New fields can be added in minor versions without breaking consumers.

Accessor return types are conservative (borrows tied to the handle, not `&'static`). This allows internal storage changes (e.g. switching from static slices to `Arc`-backed data) without breaking the public API.

### Shape mapping
`FieldShape` describes the closed/emitted shape, not the raw Rust field type. Examples:

- `bool` / `u64` / `i32` / `f64` / `String` / `Vec<u8>` lower to the corresponding `Known(KnownShape::..)` variant.
- `Timer` lowers to `Known(U64)`.
- `Option<Duration>` lowers to `Optional(Known(U64))`.
- `Vec<String>` and `&[String]` lower to `List(Known(String))`.
- `Vec<Option<String>>` lowers to `List(Optional(Known(String)))`.
- `Flex<(String, u64)>` lowers to `Flex { key: String, value: Known(U64) }`.
- `Flex<(String, Option<Duration>)>` lowers to `Flex { key: String, value: Optional(Known(U64)) }`.

`#[metrics(value)]` newtypes lower to their wrapped scalar's shape when the wrapped type is macro-known. `#[metrics(value)] struct Percent(u8)` lowers to `Known(U8)`. Newtypes wrapping user `Value` types fall through to `Opaque`.

The macro recognises one layer of `Optional` inside `List` or `Flex.value`. Deeper combinations (`Vec<Vec<T>>`, `Vec<Flex<..>>`, `Flex<(String, Vec<T>)>`, `Option<Option<T>>`) lower to `FieldShape::Opaque`; the descriptor enum can represent arbitrary nesting, the macro's syntactic recognition is what is currently restricted.

Flattening an `Option<SubEntry>` into a parent entry propagates optionality to each flattened field. If `SubEntry { baz: Option<usize> }` is flattened through an `Option<SubEntry>`, the descriptor lists `baz: Optional(Known(U64))`. `Optional` wraps the emit-or-not decision; it is not re-stacked.

`#[metrics(ignore)]` fields are not part of the descriptor. They do not emit, do not close, and do not appear in `fields()`.

### The Opaque shape

A field whose closed shape is `FieldShape::Opaque` is fully functional through `Entry::write` (every `Value` impl works; EMF and JSON handle it fine), but descriptor-aware sinks that selected it via a flag have no wire-level shape guarantee for it. Typical sinks skip opaque fields with a diagnostic and continue.

The most common current Opaque case is distribution-typed fields: `metrique_aggregation::Histogram<T>`, `SharedHistogram<T>`, and user-defined types that emit multiple `Observation`s with the `Distribution` flag. The descriptor has no way to represent "this field emits 0..N observations of an inner scalar type." Such fields are safe to use on EMF/JSON sinks today. Tagging them for a descriptor-aware sink produces a diagnostic and skips the field on that sink; see "Future evolution" for the planned `FieldShape::Distribution` variant.

Users who want custom types to flow through descriptor-aware sinks should use `#[metrics(value)]` newtypes over a known scalar.

## Descriptor lookup

The `Entry` trait has a defaulted method:

```rust
fn descriptors(&self) -> Descriptors<'_> { Descriptors::Unavailable }
```

Macro-derived entries override this to yield one or more descriptors. Composed entries (like `AggregationResult`) yield multiple descriptors in write order, one per logical segment. Hand-written entries keep the default (empty iterator). `BoxEntry` forwards the call through its dynamic dispatch layer.

`DescriptorRef` is the primary sink-facing interface. It exposes field data through `FieldView`:

```rust
for desc in entry.descriptors() {
    for field in desc.fields() {
        let parts = field.name_parts();  // prefix chain + base name, zero allocation
        let base = field.base_name();    // just the field name
        let flags = field.flags();         // resolved with defaults applied
        let shape = field.shape();
        let unit = field.unit();
    }
}
```

Sinks key their per-segment caches on `DescriptorId`. For simple entries (one descriptor), a single id suffices. For composed entries (multiple descriptors), sinks cache per-segment or use the sequence of ids as a composite key. `DescriptorId` incorporates the base descriptor pointer plus prefixes, so the same child struct with different flatten-site prefixes produces different ids.

Extending `Entry` rather than introducing a separate trait keeps descriptor lookup on the path users already know, keeps `BoxEntry` forwarding natural, and avoids growing the object-safety surface.

### Entry enums

Each enum variant gets its own static descriptor containing only that variant's fields (plus the tag field if present). The `descriptors()` method matches on self and yields the active variant's descriptor. Different variants produce different `DescriptorId`s. The descriptor name includes the variant (e.g., `"MyEnum::Read"`).

For variants with flatten fields, the variant's base descriptor is followed by the flatten children's descriptors (same chaining pattern as structs). A generated enum iterator type (same pattern as sample_group) unifies the different return types across match arms.

Sinks see different descriptor sequences depending on which variant is active. Each segment has its own `DescriptorId`, so per-segment caching works naturally. Sinks that want a single cache key for the whole entry can hash the sequence of ids.

### Aggregated entries

`AggregationResult` writes key fields then aggregated fields. Its `descriptors()` implementation chains the key entry's descriptor followed by the aggregated entry's descriptor. Both are generated by `#[metrics]` on the respective structs; no additional descriptor generation is needed in the aggregate macro.

Sinks walking `descriptors()` see two segments in write order: key fields first, aggregated fields second. Each segment's descriptor carries its own flags, units, and field names. Flatten on key fields is rejected at compile time.

### `Entry::write` order contract

The metrique macro emits exactly one `EntryWriter::value(name, ..)` callback per `FieldDescriptor`, in the same order as the fields listed in each descriptor returned by `descriptors()`. For composed entries, each descriptor covers a contiguous segment of the write output; consumers walk descriptors in sequence, consuming fields from each.

Multi-element fields (`Vec<T>`, `Flex<(String, T)>`, and similar) still produce exactly one `value()` callback per `FieldDescriptor`. The multiplicity is handled inside the `Value` impl, which the adapter's `ValueWriter` observes through `ValueWriter::values()` (for `Vec<T>` / `[T]`) or similar dispatch methods. Descriptor-aware sinks that want typed encoding for these fields override the corresponding `ValueWriter` method; the default implementations collapse multi-element data into a single scalar (comma-joined string for `values()`), which is a valid but lossy fallback.

The contract is guaranteed by construction for macro-derived entries (the macro emits both from the same iteration). A debug-mode check inside metrique's test harness validates correspondence; CI tests assert it on every release. Hand-written entries that ship a descriptor (a deferred feature) must uphold the same correspondence.

## Field flags

Sinks define flag types in their own crate. Any type works as a flag; the macro does not interpret flag identity beyond equality.

```rust
// Struct-scope default:
#[metrics(default_flags(audit::Export))]
#[metrics(default_flags(skip(audit::Export)))]

// Field override:
#[metrics(flags(audit::Export))]
#[metrics(flags(skip(audit::Export)))]
```

Each field's flags list contains only the flags that are present. `skip(T)` suppresses a default, resulting in T not appearing in the list.

### Resolution order

Flags are resolved within a single struct. They do not propagate to flattened children.

1. **Field-level `flags(T)`**: applies T to this field.
2. **Field-level `flags(skip(T))`**: suppresses T from the struct's `default_flags`.
3. **Struct-level `default_flags(T)`**: applies T to all direct fields that don't mention T.

Flattened children manage their own flags independently. This matches the scoping of container-level `prefix`, which also does not propagate to flattened children.

`#[metrics(tag(...))]` on entry enums (the entry-enum variant tag) is unchanged and unrelated.

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│ COMPILE TIME: metrique macro                                │
│                                                             │
│ For each entry type:                                        │
│   4 static EntryDescriptors (one per name style)            │
│   impl descriptors() -> chains own + flattened children     │
│   For enums: per-variant statics + enum iterator type       │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ RUNTIME: sink receives BoxEntry                             │
│                                                             │
│  descriptor-aware sink calls entry.descriptors():           │
│    yields one or more DescriptorRef segments in write order  │
│                                                             │
│  Simple struct:    [own fields]                             │
│  With flatten:     [own fields] [child1] [child2] ...       │
│  AggregationResult: [key fields] [aggregated fields]        │
│  Enum variant:     [variant's fields] [variant's children]  │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ RUNTIME: sink walks segments + Entry::write in parallel     │
│                                                             │
│  for desc in entry.descriptors():                           │
│    for field in desc.fields():                              │
│      check field.flags() -> encode or skip                   │
│      consume next value from Entry::write stream            │
└─────────────────────────────────────────────────────────────┘
```

## Units

Sinks decide how to surface units: a field-name suffix, a schema-level annotation, a separate metadata stream, whatever fits the wire format. Metrique reports units once, structurally, in the descriptor, so sinks do not have to infer them.

## Flex and List

`Flex<(String, T)>` lowers to `FieldShape::Flex { key: StringShape::String, value: .. }`.

`Vec<T>` / `[T]` / `&[T]` lower to `FieldShape::List(..)`.

One descriptor entry regardless of runtime cardinality. Sinks that understand `Flex` or `List` can register one schema field for the whole collection; sinks that do not can still fall back to per-element emission through `Entry::write`.

The inner shape may be `Known(_)` or `Optional(Known(_))` in the initial release.

## Interaction with existing `#[metrics(..)]` attributes

- **`rename_all`, `name`, `prefix`**: descriptor field names reflect the post-rename, post-prefix name that `Entry::write` emits.
- **`#[metrics(timestamp)]`**: timestamp fields are excluded from the field list and exposed separately.
- **`#[metrics(ignore)]`**: excluded from the descriptor entirely.
- **`#[metrics(subfield)]`**: subfield structs get their own descriptor, chained by the parent.
- **`flatten` / `flatten_entry`**: both chain the child's descriptor segments after the parent's own.
- **`#[metrics(value)]` newtypes**: lower to their wrapped type's shape when macro-known.

## Flatten descriptor mechanics

When a parent flattens a child, the parent's `descriptors()` chains the child's descriptor segments after its own. Prefixes and default flags from the flatten site are applied as modifiers on the child's `DescriptorRef`.

### How flatten propagates naming

When a parent flattens a child, two things propagate to the child's descriptor:

**Name style.** The parent's `rename_all` determines which of the child's pre-computed name variants is used. Each entry has a static descriptor per name style; the parent selects the one matching its own convention.

**Prefix.** A flatten-site prefix (e.g., `#[metrics(flatten, prefix = "http_")]`) is prepended to the child's field names. Nested prefixes stack. Container-level prefixes do not propagate ([#160](https://github.com/awslabs/metrique/issues/160)).

**Flags do not propagate.** Each struct manages its own flags via `default_flags` and per-field `flags(...)`. Container-level `default_flags` does not propagate to flattened children, matching the scoping of container-level `prefix`.

## Validation

Validation happens in two places.

### Compile-time (at macro expansion)

Intrinsic to the system and independent of any specific sink:

```rust
// flags(T) and flags(skip(T)) on the same field
#[metrics(flags(audit::Export), flags(skip(audit::Export)))]
request_id: String,
// -> error: conflicting flags

// default_flags(T) and default_flags(skip(T)) on the same struct
#[metrics(default_flags(audit::Export), default_flags(skip(audit::Export)))]
struct Bad;
// -> error: conflicting defaults
```

Sink-specific diagnostics (e.g. a sink-specific flag on an unsuitable `FieldShape`, an opaque value selected for a sink flag) are produced at runtime by the sink when it first sees a descriptor.

### First-use (descriptor-local, per descriptor)

The first time a descriptor-aware sink encounters a given descriptor (keyed on `DescriptorId`), it can walk the descriptor for self-contradictions its wire format does not support. The sink decides the error policy (`debug_assert!` + log, log only, silent skip, etc.).

### What is not validated

- **Flag semantics across crates.** The macro cannot know that `alice::X` and `bob::X` in different crates "mean the same thing." Flag identity is nominal.
- **Cross-entry invariants.** The descriptor describes one entry type.
- **Value validity.** Whether a field's value is in range, non-empty, etc., is outside this system; metrique's normal value validation applies.

## Binary cost

Per macro-derived struct entry type:

- 4 static `EntryDescriptor`s (one per name style), each with a slice of `FieldDescriptor`s.
- One slice of `FieldFlag` per field (shared across all 4 styles, since flags don't vary by name style).
- Per-field name strings (one per style, so 4x the name storage).

Per enum entry type: 1 static per variant, hardcoded to the enum's own rename_all style. (Struct entries have 4 statics for style propagation; enum variants do not yet support this.)

Ballpark: a ten-field struct with a couple of flags per field is roughly 2 KB of `.rodata` (4x the single-style cost). One-time cost per entry type, not per instantiation. No runtime allocation. Sinks that never call `descriptors()` pay nothing beyond their existing costs.

## Future evolution

Short list of things explicitly left out of the initial design that fit the system cleanly:

- **Typed source extraction.** See the appendix below. Would let sinks pull a typed structural snapshot (timestamp, task id, correlation id, ...) out of the closed entry before encoding fields. Deferred pending a concrete second consumer (OTEL, a richer dial9 integration).
- **Hand-written `Entry` impls opting into descriptors** via a `DescribeEntry` trait users implement by hand; same mechanism macro-derived entries use internally. Would require promoting metrique's hidden macro-only constructors to a public surface.
- **`FieldShape::Distribution(KnownShape)`** for distribution-typed fields (`Histogram<T>`, `SharedHistogram<T>`, and user types that emit many `Observation`s). Depends on a `DescribeValue` trait so value types can self-describe as distribution-shaped.
- **Nested container recognition beyond one optional layer.** `Vec<Vec<T>>`, `Vec<Flex<..>>`, `Flex<(String, Vec<T>)>`, and double-optional all fall through to `Opaque` today; the descriptor enum accepts them, the macro's syntactic recognition just does not. Relaxing is an additive macro change.
- **`#[metrics(entry_name = "...")]`** attribute for overriding the canonical entry name.
- **`no_write` attribute** for fields that participate in close but not in `Entry::write`. Deferred until a concrete consumer needs it; the deferred source system is the likely trigger.
- **Cross-process `DescriptorId` stability** via a content-hash accessor. Deferred until a consumer needs cross-process schema correlation.
- **A compile-time generated per-sink wire plan**, for sinks that want to skip runtime `Entry::write` dispatch entirely.

## Appendix: possible evolution, typed source extraction

Not shipped in the initial release. Captured here so future consumers (OTEL, a future richer dial9 integration, privacy-tier sinks) can evaluate whether it fits their needs.

Motivation: a typed source-extraction system would give sinks two capabilities that are not available in the initial design.

1. **Lifting structured envelope metadata out of the closed entry before encoding fields.** Examples: a tracing sink wants a monotonic timestamp + task id to put in the event header; an OTEL sink wants a trace id + span id; a privacy sink wants a tenant id. Today, a sink either reads those values by field-name convention or identifies them via a sink-specific field tag and walks the descriptor on first use. A typed `desc.source::<C>(entry)` API gives direct, type-checked access.
2. **Earlier validation.** The source system's optional `register_descriptor` hook lets sinks discover, at program-startup time, every descriptor in the binary declaring a given source tag. Sinks can then validate (once, at startup) that every entry carrying their source tag is shaped correctly, rather than validating lazily on first use as the initial design requires. For sinks that care about "every wrong declaration fails loudly at startup," this is the difference between a test run surfacing a problem and the first production request surfacing it.

A typed source-extraction system would add:

- A user-facing `#[metrics(source(T))]` attribute on a struct or field, declaring that the entry carries structural data of kind `T`.
- A `SourceTag` trait implemented by the sink's crate on its tag type `T`, carrying a typed `Snapshot` associated type:

  ```rust
  pub trait SourceTag: Any + Send + Sync + 'static {
      type Snapshot: Any + Send;
      fn register_descriptor(_reg: SourceRegistration) {}
  }
  ```

- A `desc.source::<C: SourceTag>(entry: &dyn Any) -> Option<C::Snapshot>` API on the descriptor, returning a typed snapshot.
- An optional `register_descriptor` hook that lets a sink discover, at program-startup time, every descriptor in the binary declaring its source tag. Backed by a link-time mechanism (e.g. `linkme`) behind the hook, so the public API does not pin the mechanism.
- A `no_write` field attribute, so source-carrying fields can be retained in the closed value without appearing in normal emission.

Forward-compat: users of the V1 tag-based shape do not need to migrate when the source system lands. The `#[metrics(source(T))]` attribute would be additive; existing declarations continue to work.

## Appendix: example consumers

Very high level; each concrete sink has its own design.

**dial9 (tracing sink).** Defines `dial9::Context` (field flag marking context fields), `dial9::Emit` (field tag), `dial9::Interned` (field tag). Reads context (worker id, task id, start and end monotonic timestamps) by walking the descriptor for fields tagged `Context`. See `dial9-tokio-telemetry/design/metrique-integration.md`.

**OTEL sink (hypothetical).** Would define `otel::InSpan` (field tag) and mark context fields similarly, or push for the typed source-extraction appendix to move in-scope.

**Custom user sinks.** Teams can add their own flag types in their own crates with no changes to metrique. Examples: a privacy-tiered export sink with `privacy::Public` / `privacy::Internal`, a metrics-rs bridge with `metricsrs::Export`.
