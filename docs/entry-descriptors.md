# Entry descriptors and field tags

> **Status: partially implemented.** Field tags, descriptors, and flatten chaining are implemented. Field shapes are deferred (all Opaque).

A small system on top of metrique's existing `Entry` / `Value` / `CloseValue` traits that lets sinks introspect the structure of macro-derived entries.

Two pieces, both opt-in for sinks:

- An **entry descriptor** that describes a macro-derived entry's closed shape: ordered fields, their tags, optionality, lists, dynamic-key maps, units, canonical entry name, and an optional timestamp field.
- A **field tag** system that lets sinks define their own static opt-in markers and lets users apply them at struct or field scope.

None of this changes the existing `Entry`, `Value`, or `CloseValue` traits. Sinks that do not call `Entry::descriptors()` pay nothing.

## Glossary

- **Entry descriptor** (`EntryDescriptor`): a metrique-emitted description of a macro-derived entry's closed shape. Sinks read it to learn what fields the entry can emit, in what order, with what tags and units, and what the entry is canonically called.
- **Field tag**: a user-defined marker type (e.g. `audit::Export`, `dial9::Emit`) that a sink crate declares and that users apply to fields via `#[metrics(field_tag(T))]`. Sinks read tags off the descriptor to decide per-field behaviour. Metrique does not interpret tag identity.
- **`default_field_tag` / `field_tag`**: struct-level and field-level attributes for applying tags. `skip(T)` is an argument form that inverts a default. Flatten sites may carry `field_tag(...)` that propagates to flattened children as a default.
- **`FieldShape`**: the closed/emitted shape of a field (scalar, optional, list, dynamic-key map, or opaque). Describes what the sink will see, not the raw Rust type.
- **`DescriptorRef`**: the handle yielded by `Entry::descriptors()`. Provides field access via `FieldView`, carries a stable `DescriptorId` for cache keying.
- **`DescriptorId`**: an opaque identifier for a descriptor, stable within a single process lifetime. Used by sinks to cache derived data.

## What it enables

- Sinks can inspect the complete set of fields an entry can emit, including optional fields and dynamic maps, without observing multiple live emissions.
- Sinks can declare per-field opt-in via tags users apply to their entries without sink-specific newtypes on field values.
- First-class units in the descriptor, surfaced however each sink prefers.
- All of the above after `BoxEntry` erasure.

Sinks that do not call `Entry::descriptors()` pay nothing at runtime.

## At a glance

Here is the end-to-end shape from a user's perspective. The example uses a made-up `audit` sink to keep the mechanics generic.

```rust
// --- in the `audit` sink crate ---

// The audit sink defines a static marker type. Users tag fields with it
// to opt them into the audit stream.
pub struct Export;

// --- in the application ---

use audit::Export;

// The struct declares `Export` as the default for all its fields; individual
// fields can override with `field_tag(skip(Export))`.
#[metrics(default_field_tag(Export))]
struct RequestMetrics {
    // `request_id` inherits the struct default: tagged Export.
    request_id: String,
    // `operation` also inherits the struct default.
    operation: &'static str,
    // `debug_blob` opts out: not in the audit payload.
    #[metrics(field_tag(skip(Export)))]
    debug_blob: String,
}
```

The macro generates (in addition to the existing `Entry` impl) an `EntryDescriptor` describing three fields: `request_id` (tag: present(Export), shape: String), `operation` (tag: present(Export), shape: String), `debug_blob` (tag: absent(Export), shape: String). The descriptor's canonical name is `"RequestMetrics"`.

An audit sink reads the descriptor at first-use per entry type, walks `Entry::write` in descriptor order, and for each value consults the tag map on the descriptor to decide whether to emit that field to its wire format.

## The descriptor model

The descriptor types live in `metrique_writer_core::descriptor`. The module defines:

- `EntryDescriptor` with accessors: `name() -> &str`, `fields() -> &[FieldDescriptor]`, `timestamp() -> Option<&TimestampDescriptor>`
- `FieldDescriptor` with accessors: `name() -> &str`, `tags() -> &[ResolvedFieldTag]`, `shape() -> FieldShape<'_>`, `unit() -> Option<Unit>`
- `TimestampDescriptor` with accessor: `name() -> &str`
- `FieldShape<'a>` enum: `Known(KnownShape)`, `Optional(ShapeRef<'a>)`, `Flex { key: StringShape, value: ShapeRef<'a> }`, `List(ShapeRef<'a>)`, `Opaque`
- `KnownShape` enum: `Bool`, `U8`..`U64`, `I8`..`I64`, `F32`, `F64`, `String`, `Bytes`
- `StringShape` enum: `String`
- `ShapeRef<'a>` with accessor: `get() -> &FieldShape<'a>`
- `ResolvedFieldTag` with accessors: `tag_id() -> TypeId`, `state() -> FieldTagState`
- `FieldTagState` enum: `Present`, `Absent`
- `DescriptorRef<'a>` with accessors: `get() -> &EntryDescriptor`, `id() -> DescriptorId`
- `DescriptorId`: opaque, `Copy + Eq + Hash`, stable within a process lifetime

### Forward compatibility

Descriptor enums (`FieldShape`, `KnownShape`, `StringShape`, `FieldTagState`) are `#[non_exhaustive]`. Consumers matching on them need a `_` arm; new variants are additive. Metrique can add variants in a minor version without breaking existing match sites, but consumers that want to *use* a new variant will need to update their code. This is by design: wire encoders (the dominant consumer) must explicitly opt into encoding new shapes.

Descriptor structs (`EntryDescriptor`, `FieldDescriptor`, `TimestampDescriptor`, `ResolvedFieldTag`, `DescriptorRef`) have private fields and accessor methods. Metrique can add fields to the structs across minor versions without breaking consumer code.

All accessor methods return borrows tied to `&self`, not `&'static`. This lets metrique change internal storage (e.g. from `&'static` slices today to `Arc`-owned data in a future enum-per-variant release) without breaking consumers. Consumers that need a longer-lived copy of a name or slice allocate from the borrow as needed.

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

### The Opaque escape hatch

A field whose closed shape is `FieldShape::Opaque` is fully functional through `Entry::write` (every `Value` impl works; EMF and JSON handle it fine), but descriptor-aware sinks that selected it via a tag have no wire-level shape guarantee for it. Typical sinks skip opaque fields with a diagnostic and continue.

The most common current Opaque case is distribution-typed fields: `metrique_aggregation::Histogram<T>`, `SharedHistogram<T>`, and user-defined types that emit multiple `Observation`s with the `Distribution` flag. The descriptor has no way to represent "this field emits 0..N observations of an inner scalar type." Such fields are safe to use on EMF/JSON sinks today. Tagging them for a descriptor-aware sink produces a diagnostic and skips the field on that sink; see "Future evolution" for the planned `FieldShape::Distribution` variant.

Users who want custom types to flow through descriptor-aware sinks should use `#[metrics(value)]` newtypes over a known scalar.

## Descriptor lookup

The `Entry` trait has a defaulted method:

```rust
fn descriptors(&self) -> impl Iterator<Item = DescriptorRef<'_>> { std::iter::empty() }
```

Macro-derived entries override this to yield one or more descriptors. Composed entries (like `AggregationResult`) yield multiple descriptors in write order, one per logical segment. Hand-written entries keep the default (empty iterator). `BoxEntry` forwards the call through its dynamic dispatch layer.

`DescriptorRef` is the primary sink-facing interface. It exposes field data through `FieldView`:

```rust
for desc in entry.descriptors() {
    for field in desc.fields() {
        let parts = field.name_parts();  // prefix chain + base name, zero allocation
        let base = field.base_name();    // just the field name
        let tags = field.tags();         // resolved with defaults applied
        let shape = field.shape();
        let unit = field.unit();
    }
}
```

Sinks key their per-segment caches on `DescriptorId`. For simple entries (one descriptor), a single id suffices. For composed entries (multiple descriptors), sinks cache per-segment or use the sequence of ids as a composite key. `DescriptorId` incorporates the base descriptor pointer plus any modifiers (prefix, default tags), so the same child struct with different flatten-site prefixes produces different ids.

Extending `Entry` rather than introducing a separate trait keeps descriptor lookup on the path users already know, keeps `BoxEntry` forwarding natural, and avoids growing the object-safety surface.

### Entry enums

For enums without flatten fields in any variant, `descriptors()` yields a single descriptor containing the union of all variant fields (deduplicated by name) plus the tag field.

For enums with flatten fields, `descriptors()` dispatches per-variant: each variant yields the shared base descriptor (union of non-flatten fields) followed by that variant's flatten children's descriptors. An enum iterator type (generated per entry, same pattern as sample_group) unifies the different return types across variants.

Sinks see different descriptor sequences depending on which variant is active. Each segment has its own `DescriptorId`, so per-segment caching works naturally. Sinks that want a single cache key for the whole entry can hash the sequence of ids.

### Aggregated entries

`AggregationResult` writes key fields then aggregated fields. Its `descriptors()` implementation chains the key entry's descriptor followed by the aggregated entry's descriptor. Both are generated by `#[metrics]` on the respective structs; no additional descriptor generation is needed in the aggregate macro.

Sinks walking `descriptors()` see two segments in write order: key fields first, aggregated fields second. Each segment's descriptor carries its own tags, units, and field names. Flatten on key fields is rejected at compile time.

### `Entry::write` order contract

The metrique macro emits exactly one `EntryWriter::value(name, ..)` callback per `FieldDescriptor`, in the same order as the fields listed in each descriptor returned by `descriptors()`. For composed entries, each descriptor covers a contiguous segment of the write output; consumers walk descriptors in sequence, consuming fields from each.

Multi-element fields (`Vec<T>`, `Flex<(String, T)>`, and similar) still produce exactly one `value()` callback per `FieldDescriptor`. The multiplicity is handled inside the `Value` impl, which the adapter's `ValueWriter` observes through `ValueWriter::values()` (for `Vec<T>` / `[T]`) or similar dispatch methods. Descriptor-aware sinks that want typed encoding for these fields override the corresponding `ValueWriter` method; the default implementations collapse multi-element data into a single scalar (comma-joined string for `values()`), which is a valid but lossy fallback.

The contract is guaranteed by construction for macro-derived entries (the macro emits both from the same iteration). A debug-mode check inside metrique's test harness validates correspondence; CI tests assert it on every release. Hand-written entries that ship a descriptor (a deferred feature) must uphold the same correspondence.

## Field tags

Sinks define tag types in their own crate. Any type works as a tag; the macro does not interpret tag identity beyond equality.

```rust
// Struct-scope default:
#[metrics(default_field_tag(audit::Export))]
#[metrics(default_field_tag(skip(audit::Export)))]

// Field override:
#[metrics(field_tag(audit::Export))]
#[metrics(field_tag(skip(audit::Export)))]
```

Each field/tag pair resolves to one of `present`, `absent`, or `unspecified`. Only `present` and `absent` (explicit user decisions) appear in the descriptor's `ResolvedFieldTag` list; `unspecified` is the absence of any entry.

### Resolution order

From most-specific to least-specific:

1. **Field-level `field_tag(T)` on the child field** wins.
2. **Struct-level `default_field_tag(T)` on the child struct** wins over a flatten-site tag.
3. **`field_tag(T)` on a flatten site** propagates to the flattened children as a default, overriding the grandparent default.
4. **Parent struct-level `default_field_tag(T)`** fills anything still unspecified.

`skip(T)` is an argument form, not a separate attribute.

`#[metrics(tag(...))]` on entry enums (the entry-enum variant tag) is unchanged and unrelated.

Full resolution rules including worked inheritance and flatten cases are documented alongside the macro's other field attributes.

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│ COMPILE TIME: metrique macro                                │
│                                                             │
│ For each macro-derived entry:                               │
│   impl Entry for ClosedX (as today)                         │
│   static EntryDescriptor                                    │
│   impl Entry::descriptors() yielding DescriptorRef          │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ RUNTIME: construction                                       │
│                                                             │
│ Fields populated normally.                                  │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ RUNTIME: append-on-drop / close                             │
│                                                             │
│ CloseValue closes all fields.                               │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ RUNTIME: BackgroundQueue / tee                              │
│                                                             │
│ BoxEntry flows to one or more sinks.                        │
│                                                             │
│  ├── descriptor-unaware sink                                │
│  │     calls Entry::write; never calls descriptors()        │
│  │                                                          │
│  └── descriptor-aware sink                                  │
│        calls entry.descriptors()                            │
│          empty  -> skip (hand-written entry, opaque)        │
│          yields -> first-use structural checks, cache on    │
│                    id(), then proceed                        │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ RUNTIME: inside a descriptor-aware sink                     │
│                                                             │
│ entry.write(SinkWriter { descs, tag: audit::Export }):      │
│   walks Entry::write; the adapter consults descriptors      │
│   (cached by DescriptorId) to decide per-field behaviour:   │
│     - field tagged with the sink's tag -> encode            │
│     - otherwise                         -> ignore           │
└─────────────────────────────────────────────────────────────┘
```

## Units

Sinks decide how to surface units: a field-name suffix, a schema-level annotation, a separate metadata stream, whatever fits the wire format. Metrique reports units once, structurally, via `FieldDescriptor::unit()`, so sinks do not have to infer them.

## Flex and List

`Flex<(String, T)>` lowers to `FieldShape::Flex { key: StringShape::String, value: .. }`.

`Vec<T>` / `[T]` / `&[T]` lower to `FieldShape::List(..)`.

One descriptor entry regardless of runtime cardinality. Sinks that understand `Flex` or `List` can register one schema field for the whole collection; sinks that do not can still fall back to per-element emission through `Entry::write`.

The inner shape may be `Known(_)` or `Optional(Known(_))` in the initial release.

## Interaction with existing `#[metrics(..)]` attributes

- **`rename_all` and `name` / `name_exact`**: `FieldDescriptor::name()` returns the post-rename, post-override name that `Entry::write` emits. `EntryDescriptor::name()` returns the raw Rust struct name (a future `#[metrics(entry_name = "...")]` attribute will let users override).
- **`prefix` / `exact_prefix`**: applied to field names before they land in the descriptor.
- **`#[metrics(timestamp)]`**: timestamp fields are excluded from `fields()` and exposed via `EntryDescriptor::timestamp()`. They emit via `EntryWriter::timestamp`, not `EntryWriter::value`, so they are correctly not part of the `fields()` walk.
- **`#[metrics(ignore)]`**: fields are excluded from the descriptor entirely. No `Entry::write` callback, no `FieldDescriptor`.
- **`#[metrics(subfield)]` / `#[metrics(subfield_owned)]`**: subfield structs get their own descriptor. When flattened into a parent, the parent's `descriptors()` chains the subfield's descriptor after its own, matching write order.
- **`flatten` vs `flatten_entry`**: both produce flattened fields in the parent descriptor identically. The distinction is about how metrique resolves the nested struct internally (inflection, prefixes).
- **`#[metrics(value)]` newtypes**: lower to their wrapped type's shape when macro-known. See "Shape mapping" above.

## Flatten descriptor mechanics

When a parent flattens a child, the parent's `descriptors()` chains the child's descriptor segments after its own. Prefixes and default tags from the flatten site are applied as modifiers on the child's `DescriptorRef`.

### Name style resolution

Each entry type has 4 static descriptors (one per name style). A `__metrique_descriptor(style: u8)` inherent method selects the right one. Named constants (`STYLE_PRESERVE`, `STYLE_PASCAL`, `STYLE_SNAKE`, `STYLE_KEBAB`) are the single source of truth. The struct's own `descriptors()` uses its own `rename_all` index. Flatten chains call the child's `descriptors()` with the parent's concrete NS (from `make_ns(root_attrs.rename_all, ...)`), matching the write path's name style propagation.

### Prefix handling

Flatten-site prefixes (e.g., `#[metrics(flatten, prefix = "http_")]`) are precomputed at macro time in the parent's name style and applied via `.with_prefix()` on the child's `DescriptorRef`. Nested flatten-site prefixes stack via `SmallVec`. `FieldView::name_parts()` yields all prefixes (outermost first) then the base name. Sinks concatenate the parts to get the full field name.

Container-level prefixes (on the struct itself) are baked into the struct's own static descriptor field names. They do NOT propagate to flattened children (see [#160](https://github.com/awslabs/metrique/issues/160)).

### Tag propagation through flatten

When a parent flattens a child, the macro merges the flatten-site `field_tag` attributes with the parent's `default_field_tag` into a single defaults slice (flatten-site wins over parent default for the same tag id). This slice is applied via `.with_default_tags()` on the child's `DescriptorRef`.

At read time, `FieldView::tags()` resolves precedence by walking layers: the child's own tags (baked in its static) win; then each default layer is walked innermost-first, skipping tag ids already present. This gives the full resolution order:

1. Child field-level `field_tag` (baked) wins
2. Child struct-level `default_field_tag` (baked) wins
3. Flatten-site `field_tag` (in merged defaults) fills gaps
4. Parent `default_field_tag` (in merged defaults, lowest priority) fills remaining gaps

## Validation

Validation happens in two places.

### Compile-time (at macro expansion)

Intrinsic to the system and independent of any specific sink:

```rust
// field_tag(T) and field_tag(skip(T)) on the same field
#[metrics(field_tag(audit::Export), field_tag(skip(audit::Export)))]
request_id: String,
// -> error: conflicting field tags

// default_field_tag(T) and default_field_tag(skip(T)) on the same struct
#[metrics(default_field_tag(audit::Export), default_field_tag(skip(audit::Export)))]
struct Bad;
// -> error: conflicting defaults
```

Sink-specific diagnostics (e.g. a sink-specific tag on an unsuitable `FieldShape`, an opaque value selected for a sink tag) are produced at runtime by the sink when it first sees a descriptor.

### First-use (descriptor-local, per descriptor)

The first time a descriptor-aware sink encounters a given descriptor (keyed on `DescriptorId`), it can walk the descriptor for self-contradictions its wire format does not support. The sink decides the error policy (`debug_assert!` + log, log only, silent skip, etc.).

### What is not validated

- **Tag semantics across crates.** The macro cannot know that `alice::X` and `bob::X` in different crates "mean the same thing." Tag identity is nominal.
- **Cross-entry invariants.** The descriptor describes one entry type.
- **Value validity.** Whether a field's value is in range, non-empty, etc., is outside this system; metrique's normal value validation applies.

## Binary cost

The initial release adds, per macro-derived entry type:

- One `static EntryDescriptor` in `.rodata`.
- One slice of `FieldDescriptor`s (one per emitted field).
- One or more slices of `ResolvedFieldTag` per field (only for tags that resolved to `Present` or `Absent` explicitly).
- Small per-field constants for names, shapes, and units.

Ballpark: a ten-field entry with a couple of tags per field and some nested shapes fits in about 500-1500 bytes of `.rodata`. One-time cost per entry type, not per instantiation. No runtime allocation. Sinks that never call `Entry::descriptors()` pay nothing beyond their existing costs.

## Future evolution

Short list of things explicitly left out of the initial design that fit the system cleanly:

- **Typed source extraction.** See the appendix below. Would let sinks pull a typed structural snapshot (timestamp, task id, correlation id, ...) out of the closed entry before encoding fields. Deferred pending a concrete second consumer (OTEL, a richer dial9 integration).
- **Hand-written `Entry` impls opting into descriptors** via a `DescribeEntry` trait users implement by hand; same mechanism macro-derived entries use internally. Would require promoting metrique's hidden macro-only constructors to a public surface.
- **Per-variant descriptors for entry enums.** A future `Entry::descriptors()` impl on an enum could yield a different `DescriptorRef` per variant. `DescriptorRef` is opaque today specifically to leave this open; a `Shared(Arc<..>)`-backed variant of the handle would ship with that work.
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

The trade-offs were worked through in earlier revisions of this design and are captured in the review doc's "Deferred: typed source extraction" section. The short version:

- Wiring the hook into the `SourceTag` trait means metrique's macro emits one registration per `source(T)` declaration per descriptor whether the hook is overridden or not. Small (one pointer + linkme slot per declaration) but non-zero binary cost for every user.
- Skipping the hook entirely and keeping only the typed extraction API forgoes binary-wide discovery; sinks can still validate per-descriptor on first use.
- Skipping the whole source system and letting sinks read structural fields by tag-based marker (e.g. a `Dial9Context`-style struct whose fields carry a `dial9::Context` tag) works for the initial dial9 integration without any metrique surface beyond what is already shipped.

The initial release takes the last path. When a second consumer (OTEL, other) materialises, the design-space discussion reopens here.

Forward-compat: users of the V1 tag-based shape do not need to migrate when the source system lands. The `#[metrics(source(T))]` attribute would be additive; existing declarations continue to work.

## Appendix: example consumers

Very high level; each concrete sink has its own design.

**dial9 (tracing sink).** Defines `dial9::Context` (field tag marking context fields), `dial9::Emit` (field tag), `dial9::Interned` (field tag). Reads context (worker id, task id, start and end monotonic timestamps) by walking the descriptor for fields tagged `Context`. See `dial9-tokio-telemetry/design/metrique-integration.md`.

**OTEL sink (hypothetical).** Would define `otel::InSpan` (field tag) and mark context fields similarly, or push for the typed source-extraction appendix to move in-scope.

**Custom user sinks.** Teams can add their own tag types in their own crates with no changes to metrique. Examples: a privacy-tiered export sink with `privacy::Public` / `privacy::Internal`, a metrics-rs bridge with `metricsrs::Export`.
