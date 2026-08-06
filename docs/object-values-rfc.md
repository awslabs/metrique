# RFC: Object-valued fields

> Status: RFC (design only, no prototype)
>
> Applies to: `metrique-writer-core`, `metrique-writer`,
> `metrique-writer-format-emf`, `metrique-writer-format-json`, `metrique-macro`

For a summarized list of proposed changes, see the
[Changes checklist](#changes-checklist).

This RFC proposes a way to emit a struct field as a nested object rather than a
flat scalar. On formats that support structured values (JSON, EMF), the field is
written as a native nested object. On formats that do not, the field is dropped.
A descriptor-time warning fires once at setup to inform the user which fields
will not render. This addresses
[#355](https://github.com/awslabs/metrique/issues/355).

---

**_TLDR:_** _`metrique` is flat today, which is a feature and not an accident:
there is no ambiguity about what is a metric and what is a property, and it
discourages people from attaching unbounded structured data to metric entries.
This RFC keeps that property. Object rendering is opt-in per field through the
existing `#[metrics(format = ...)]` attribute, a nested object reuses the
`Entry` machinery rather than introducing a parallel one, and any ordinary
`#[metrics]` struct can be nested with no re-annotation. A bare struct-typed
field remains a compile error, so nothing becomes an object by accident._

---

## Motivation

`metrique` maps each field of a `#[metrics]` struct to a flat scalar. A field is
either a metric (a number with a unit and dimensions, such as a `Timer` or a
counter) or a property (a string, an identifier, some context). There is
currently no way to attach a structured, nested object to an entry.

Today, a user who wants to attach a multi-field blob of context has two options,
and neither is good:

1. Flatten every field into top-level fields. For a dynamic tree of
   units-of-work, this produces field names that are not known at compile time,
   which makes the log event difficult for a human operator to read when
   investigating a problem. Furthermore, CloudWatch Logs Insights does not
   support querying dynamic field names — it requires exact field names or
   regex-based matching on the raw log event, neither of which is practical for
   a recursive data structure whose shape is caller-defined.
2. Serialize the blob to a string by hand and emit it as a string property. This
   throws away structure that downstream systems could otherwise use.

The second option is worse than it first appears. CloudWatch Logs Insights has
no `parse_json` command, so a field emitted as a JSON-encoded string is not
queryable as structured data in that backend. In other words, stringifying is
not merely inconvenient; for a common query path it is lossy.

The concrete case that motivates this proposal is a tree of units of work whose
shape is not known at compile time. Consider a calculator service that accepts
an expression such as `(2 + 4) * (8 - 1)` and evaluates it. The request has a
top-level unit of work, and each phase of evaluation (parsing, each operation)
is itself a unit of work. The phases nest, and the set of operations is defined
by the caller's input rather than by the service author. The desired output
attaches the phase tree to the request entry as a native nested structure:

```json
{
  "Time": 123,
  "RequestId": "123e4567-e89b-12d3-a456-426614174000",
  "Phases": [
    { "Type": "parse_expr", "Duration": 11 },
    {
      "Type": "eval",
      "Duration": 22,
      "Phases": [
        {
          "Type": "multiply",
          "Duration": 33,
          "Phases": [
            { "Type": "add", "Duration": 44 },
            { "Type": "sub", "Duration": 55 }
          ]
        }
      ]
    },
    { "Type": "result_serialization", "Duration": 66 }
  ]
}
```

Each phase is an ordinary unit-of-work metric struct. The user should be able to
reuse the metric structs they already have, rather than build a parallel
mechanism to produce this tree.

## Terminology

- **Object field**: A field of a `#[metrics]` struct that is rendered as a
  nested object rather than a flat scalar.
- **Object payload**: The value written as an object. Either an ordinary
  `#[metrics]` struct or a type that hand-implements `ObjectValue`.
- **Native object**: A structured object emitted directly in a format that
  supports it (a JSON object in JSON and EMF; a self-describing value in a
  future OTel integration).

## The user experience if this RFC is implemented

This section is written as the public documentation users should be able to read
once the feature is released.

### Nesting an existing metric struct

Any `#[metrics]` struct can be nested inside another entry as an object. The
nested struct requires no new annotation; the parent field selects object
rendering with `#[metrics(format = AsObject)]`:

```rust,ignore
use metrique::unit_of_work::metrics;
use metrique::writer::value::AsObject;

#[metrics(rename_all = "PascalCase")]
struct Phase {
    r#type: &'static str,
    duration: Duration,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    request_id: &'static str,

    #[metrics(format = AsObject)]
    root_phase: Phase,
}
```

When `RequestMetrics` closes, `root_phase` is a nested object:

```json
{
  "RequestId": "...",
  "RootPhase": { "Type": "parse_expr", "Duration": 11 }
}
```

The nested object is a fresh naming scope. `Phase` applies its own `rename_all`;
the parent's field name (`root_phase`) becomes the object's key, and the phase's
field names are the object's members.

### Nesting a tree of metric structs

A recursive tree is a struct with a field holding a `Vec` of itself. The `Vec`
of objects uses `#[metrics(format = Each<AsObject>)]`:

```rust,ignore
use metrique::writer::value::{AsObject, Each};

#[metrics(rename_all = "PascalCase")]
struct Phase {
    r#type: &'static str,
    duration: Duration,

    #[metrics(format = Each<AsObject>)]
    phases: Vec<Phase>,
}
```

This produces the nested `Phases` array shown in the [Motivation](#motivation)
section, to arbitrary depth. The set of phases does not need to be known at
compile time.

### Fields inside an object are plain values, not metrics

All fields inside an object are rendered as regular JSON values. Numeric fields
are bare numbers with no unit, no dimensions, and no aggregation. String fields
are bare strings. No field inside an object is registered as a CloudWatch
metric, and no `_aws` metadata (metric directives, namespace, dimension sets) is
emitted for a nested object. A `Timer` or a counter placed inside an object
renders as its bare numeric value; its unit and dimensions are discarded.

This is structural, not advisory. The nested object writer ignores `config()`
(so EMF configuration such as namespace and dimension sets has no effect), and
the inner member `ValueWriter` strips metric semantics from numeric values
(discards unit, dimensions, and flags, writing only the raw observation). There
is no code path through which `_aws` metadata can reach a nested object.

This is the same rule that already applies to numbers inside an array (see
[#266](https://github.com/awslabs/metrique/pull/266)).

### Objects on a type you do not own

An object payload does not have to be a `#[metrics]` struct. A type that cannot
be annotated (for example, a type from another crate) can be rendered as an
object by wrapping it in a newtype and implementing both `ObjectValue` and
`ValueFormatter` by hand:

```rust,ignore
use metrique::writer::{EntryWriter, ValueWriter, value::{ObjectValue, AsObject}};
use metrique::writer::value::ValueFormatter;

struct EndpointObject(third_party::Endpoint);

impl ObjectValue for EndpointObject {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("host", &self.0.host);
        writer.value("port", &self.0.port);
    }
}

impl ValueFormatter<EndpointObject> for AsObject {
    fn format_value(writer: impl ValueWriter, value: &EndpointObject) {
        writer.object(value)
    }
}

#[metrics]
struct RequestMetrics {
    #[metrics(format = AsObject)]
    endpoint: EndpointObject,
}
```

`ObjectValue::write_object` receives the same `EntryWriter` that a hand-written
`Entry` receives, so each member is written with `writer.value(name, value)`.
Every member passed to `writer.value()` must implement `Value`; if a member does
not (for example, another nested struct), wrap it with its own `AsObject`
formatter or implement `Value` for it. The concrete `ValueFormatter` impl is
boilerplate (it calls `writer.object(value)`), but it is required so that
`L`-inference is unambiguous at the macro's use site.

### What happens on formats without native objects

On formats that support native objects (JSON, EMF), `AsObject` renders the field
as a structured object. On formats that do not, the field is **dropped** — it
does not appear in the output. A descriptor-time warning (see below) informs the
user at setup which fields will not render on which formats.

Dropping is preferable to a string fallback as the default: it avoids surprise
size inflation from unqueryable JSON blobs on flat formats, and the warning
makes the omission a conscious choice rather than a silent loss.

### A warning when a format cannot render an object

When an entry containing an object field is written to a format that does not
support native objects, the framework emits a warning once at entry-format setup
time (not per-write). The warning names the field and the format.

The mechanism is described in
[Descriptor shape and the format-capability warning](#descriptor-shape-and-the-format-capability-warning).

## How to actually implement this RFC

### An object is an `Entry`, not a new kind of value

A nested object is a visitor over named fields written into a `{ ... }` scope.
This is exactly the contract of the existing `EntryWriter`:

```rust,ignore
// metrique-writer-core/src/entry/mod.rs
pub trait EntryWriter<'a> {
    fn timestamp(&mut self, timestamp: SystemTime);
    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized));
    fn config(&mut self, config: &'a dyn EntryConfig);
}
```

Rather than introduce a parallel object-writer trait, each format provides a
small `EntryWriter` implementation that renders `value(name, v)` as
`"name": <v>` into a nested scope, and ignores `timestamp` and `config` (an
object has no timestamp and no entry-level configuration).

The single new public trait is `ObjectValue`:

```rust,ignore
// metrique-writer-core/src/value/object.rs
pub trait ObjectValue {
    /// Emit this object's members. Each `value(name, v)` call becomes a
    /// `"name": <v>` member of the object. `timestamp` and `config` calls
    /// are ignored.
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>);
}
```

`ObjectValue` is obtained two ways: the `#[metrics]` derive generates it for
every entry-producing struct, and a user hand-implements it for a type they do
not own (through a newtype, as shown above). There is deliberately no blanket
`impl<T: InflectableEntry> ObjectValue for T`: such a blanket would make the
compiler unable to prove that a hand-written `impl ObjectValue for MyNewtype`
does not overlap it, so hand-implementation and a blanket bridge are mutually
exclusive. Generating an explicit impl per type in the derive avoids the
conflict.

### One new `ValueWriter` method

`ValueWriter` gains `object`, following the same default-plus-override pattern
that `values()` established:

```rust,ignore
// metrique-writer-core/src/value/mod.rs
pub trait ValueWriter: Sized {
    // ... string, metric, error, invalid, values ...

    /// Write a nested object. Formats with native object support override
    /// this to emit a structured object. The default drops the field (emits
    /// nothing). The descriptor-time warning ensures the user knows which
    /// formats will drop.
    fn object<O: ObjectValue + ?Sized>(self, _object: &O) {}
}
```

Formats that support native objects (JSON, EMF) override `object` to render the
structure. Formats that do not inherit the default no-op.

This satisfies the tee case. A single entry written to an EMF-aggregating sink
and a non-EMF sampling sink at the same time resolves per-format at write time,
because each format supplies (or inherits) its own override. There is no
per-struct or per-sink static choice.

The EMF override writes the object into `string_fields_buf` (the plain-JSON body
of the EMF document), never into the `_aws` metrics directive. The JSON override
writes into `properties_buf`. The per-format nested object `EntryWriter` uses a
monomorphic buffer-borrowing writer (not a generic wrapper over `W`) to avoid
recursive monomorphization. Both formats reuse their existing array-element
writers to render member values as bare numbers, which is what gives object
numbers their non-metric semantics.

Every wrapper `ValueWriter` must forward `object`, exactly as it must already
forward `values`. A wrapper that does not forward will silently drop a
would-be-native object. The `metrique_require_explicit_impls` cfg gate should be
extended to cover `object` so that CI catches missing forwarding impls.

### Rendering is a formatter

Object rendering is expressed through the existing `#[metrics(format = ...)]`
mechanism. The `format` attribute value is parsed via `parse_nested_meta` (the
same approach used by `#[aggregate(strategy = Histogram<Duration>)]`), which
gives a raw `ParseStream` after the `=` and parses it as a `syn::Type`. This
allows generic syntax like `Each<AsObject>` without string quoting or turbofish,
because in type grammar `<>` is unambiguous.

A `ValueFormatter` receives the `ValueWriter` and chooses which method to call:

```rust,ignore
pub struct AsObject;

// Macro-generated per entry type (or hand-written for newtypes):
impl ValueFormatter<PhaseEntry> for AsObject {
    const SHAPE: FieldShape<'static> = FieldShape::Object;
    fn format_value(writer: impl ValueWriter, value: &PhaseEntry) {
        writer.object(value)
    }
}
```

`AsObject` is a unit struct with no blanket impl in the library. Every type that
uses it — whether derive-generated or hand-written — gets its own concrete
`Lifted` impl. This is required for two reasons:

1. A blanket `impl<O: ObjectValue> ValueFormatter<O, NotLifted> for AsObject`
   causes `E0283` inference ambiguity at the macro's use site (the compiler
   cannot choose between `Lifted` and `NotLifted` when both satisfy the bound).
2. Without a concrete `Lifted` impl, `Option`/`Box`/`Arc` auto-lifting does not
   work (the lifting blankets are keyed on `L = Lifted`).

The derive generates `impl ValueFormatter<PhaseEntry> for AsObject` per type.
Hand-implementors write the same impl. Because every impl is concrete and
`Lifted`, the lifting blankets compose automatically for all types.

### The `Each` format combinator

The recursive case requires rendering a `Vec<Phase>` as an array of objects. The
closed form is `Vec<PhaseEntry>`, where `PhaseEntry` implements
`InflectableEntry`, not `Value`. The existing `ValueWriter::values` requires its
items to implement `Value`, so it cannot take a `Vec<PhaseEntry>` directly.

`Each<F>` is a general format combinator that applies a formatter `F` to each
element of a collection and writes the results as an array via `values()`. It
joins the family of lifting types alongside `Option`, `Box`, `Arc`, and `Cow`:

```rust,ignore
pub struct Each<F>(PhantomData<F>);

impl<V, F> ValueFormatter<Vec<V>> for Each<F>
where
    F: ValueFormatter<V>,
{
    const SHAPE: FieldShape<'static> =
        FieldShape::List(ShapeRef::new(&<F as ValueFormatter<V>>::SHAPE));

    fn format_value(writer: impl ValueWriter, value: &Vec<V>) {
        let wrapped: SmallVec<[FormattedValue<_, F, _>; VALUES_INLINE_CAPACITY]> =
            value.iter().map(FormattedValue::new).collect();
        writer.values(wrapped.iter());
    }
}
```

`Each<F>` wraps each element in `FormattedValue` (which already implements
`Value`), then passes the wrapped iterator to `values()`. No separate bridge
type is needed.

Because `Each<F>` is a concrete type (not a blanket impl), it is `Lifted` by
default and composes with the existing lifting blankets. `Option<Vec<Phase>>`
with `format = Each<AsObject>` auto-lifts through the `Option` blanket.

`Each` is not specific to objects. It is a general combinator for rendering
collections where each element needs a formatter applied:

- `format = Each<AsObject>` — array of native objects
- `format = Each<AsEpochSeconds>` — array of formatted timestamps
- `format = Each<Each<AsObject>>` — nested `Vec<Vec<Phase>>`

The re-wrap-into-a-`SmallVec` step is the same pattern `ForceFlag::values`
already uses. For arrays that exceed `VALUES_INLINE_CAPACITY`, this spills to a
heap allocation per level of nesting. This is the same cost `ForceFlag` already
pays and is acceptable for the expected use case (tens of elements per level,
not thousands). The format changes required for recursion are that the
array-element writers gain native `object()` (for objects inside arrays) and the
object-member writers gain native `values()` (for arrays inside objects). Both
directions must be implemented to support arbitrary nesting.

Recursion works because `Vec` supplies the heap indirection that keeps the
recursive closed type `Sized`, and because each element's `write` calls back
into `object()`, which drives the element's `write_object`, which reaches the
next `Vec` and repeats.

### The strong guarantee: a bare struct field does not compile

The derive generates `ObjectValue` for an entry struct, but it does not generate
a `Value` impl that emits an object. A bare field such as `phase: Phase`
therefore takes the ordinary field path, which requires the field's closed type
to implement `Value`. The closed type of a `#[metrics]` struct (`PhaseEntry`)
implements `InflectableEntry`, not `Value`, so a bare struct-typed field is a
compile error. The existing `Value` `#[diagnostic::on_unimplemented]` message
already points the user toward flattening.

To render a struct as an object, the field must carry
`#[metrics(format = AsObject)]` (or `Each<AsObject>` for arrays). Nothing
becomes a nested object implicitly. This keeps the escape hatch visible at the
point of use, which is the mitigation for the "pit of success" concern raised in
[#355](https://github.com/awslabs/metrique/issues/355).

### Descriptor shape and the format-capability warning

The macro currently reports `FieldShape::Opaque` for any field that uses
`format = ...`, because the formatter's liftability parameter is not inferable
in a `const` position and the macro cannot uniformly read an arbitrary
formatter's `SHAPE`.

However, the library-shipped formatters are a known set. The macro can
**recursively pattern-match the formatter type** at compile time to derive the
shape:

- `AsObject` → `FieldShape::Object`
- `ToString` → `FieldShape::Known(KnownShape::String)`
- `Each<inner>` → `FieldShape::List(ShapeRef::new(&shape_of(inner)))`
- Anything unrecognized → `FieldShape::Opaque` (same as today)

This is a recursive walk: `Each<Each<AsObject>>` → `List(List(Object))`. The set
of recognized leaf formatters is finite (the library-shipped set); `Each` is the
structural combinator. User-defined formatters degrade to `Opaque` and do not
participate in the warning.

The warning mechanism is a first-write check: the first time a format writes an
entry whose descriptor contains `Object` (or `List` containing `Object`), it
checks whether it overrides `object()`. If not, it emits a warning naming the
field and the format. The check is guarded by a `Once`-style flag per
(entry-type, format) pair so it fires at most once. No new trait method or
registration hook is needed — the check runs inline on the first write for each
entry type.

This is a stopgap. A general descriptor-based format-capability validation
system (where formats declare their capabilities and entries are validated
against them at registration time) is the proper long-term solution; see
[#359](https://github.com/awslabs/metrique/issues/359). Once that exists, this
first-write check can be removed.

### Known limitation: the boxed entry path

The `BoxEntry` / `DynValueWriter` path
(`metrique-writer-core/src/entry/boxed.rs`) interposes a dyn-dispatch boundary
between the `Value::write` call and the real format writer. `ValueWriterFromDyn`
would inherit the default no-op for `object()`, silently dropping object fields
even when the underlying format (EMF, JSON) supports them.

This is the same class of limitation that `values()` already has today:
`ValueWriterFromDyn::values()` stringifies every element through
`StringCapture`, losing type information. Neither issue is new to this RFC.

Because `BoxEntrySink` is the default sink path (returned by
`ServiceMetrics::sink()`), this limitation affects the common case. Users who
need object fields must use a non-boxed sink path (e.g., a typed
`FlushImmediately` or `BackgroundQueue` sink). This is a UX concern and should
be addressed — but it is a pre-existing architectural limitation of the
dyn-dispatch boundary, not a regression introduced by this feature.

## Changes checklist

This RFC does not include a prototype. The list below is the proposed
implementation scope, not completed work.

### Core

- [ ] Add the `ObjectValue` trait to `metrique-writer-core`, re-exported through
      `metrique::writer::value`.
- [ ] Add `ValueWriter::object` with a default no-op (drop) implementation.
- [ ] Add `object` to the `metrique_require_explicit_impls` cfg gate so CI
      catches missing forwarding impls.
- [ ] Add the `AsObject` formatter unit struct (no library blanket impl; the
      derive and hand-impl users provide concrete `Lifted` impls per type).
- [ ] Add the `Each<F>` format combinator (`Lifted`; applies `F` per element via
      `FormattedValue`, writes via `values()`).
- [ ] Forward `object` through every wrapper `ValueWriter` (all impls covered by
      the `metrique_require_explicit_impls` gate).

### Formats

- [ ] EMF: override `object` to write into `string_fields_buf` using a
      monomorphic nested `EntryWriter`; add native `object()` to the
      array-element writer; add native `values()` to the object-member writer.
- [ ] JSON: override `object` to write into `properties_buf` using a monomorphic
      nested `EntryWriter`; add native `object()` to the array-element writer;
      add native `values()` to the object-member writer.
- [ ] LocalFormat: override `object` to write a native JSON object (LocalFormat
      is JSON-based and should support objects natively).
- [ ] Confirm numeric members inside an object render as bare numbers and are
      not registered as metrics.

### Macro

- [ ] Parse `format` via `parse_nested_meta` (same approach as
      `#[aggregate(strategy = ...)]`): extract the format value from the raw
      `ParseStream` as a `syn::Type` before darling processes the attribute, so
      generic syntax like `Each<AsObject>` works without string quoting.
- [ ] Generate `impl ObjectValue` for every entry-producing `#[metrics]` mode
      (`RootEntry`, `Subfield`, `SubfieldOwned`).
- [ ] Generate a concrete `Lifted` `impl ValueFormatter<FooEntry> for AsObject`
      per entry type, so that `Option`/`Box`/`Arc` auto-lift and `L`-inference
      succeeds.

### Descriptors and warning

- [ ] Add the `FieldShape::Object` variant (additive; `FieldShape` is
      `#[non_exhaustive]`).
- [ ] Implement recursive shape derivation in the macro's `shape_expr`:
      recognize `AsObject` → `Object`, `ToString` → `Known(String)`,
      `Each<inner>` → `List(shape_of(inner))`, else `Opaque`.
- [ ] Implement the first-write warning: on the first write of an entry type to
      a format, check the descriptor for `Object` (or `List` containing
      `Object`) and warn if the format does not override `object()`. Guard with
      a `Once`-style flag per (entry-type, format) pair. This is a stopgap
      pending general descriptor-based format-capability validation
      ([#359](https://github.com/awslabs/metrique/issues/359)).

### Tests and docs

- [ ] Cover: a nested struct; a recursive `Vec<Self>` tree two or three levels
      deep; a hand-written `ObjectValue` on a newtype with its concrete
      `ValueFormatter` impl.
- [ ] Verify that a metric-typed field inside an object (e.g. a `Timer` with a
      unit) renders as a bare number, carries no unit or dimensions, and does
      not appear in the `_aws.CloudWatchMetrics` metric directive.
- [ ] Verify that an object payload with `emf::dimension_sets` and a namespace
      configured on it produces no `_aws` metadata in the nested object output
      (the nested `EntryWriter` must discard `config()` entirely).
- [ ] Verify wrapper/dimension/force-flag combinations: a field wrapped with
      `WithUnit` or `ForceFlag` that contains an object confirms metric
      semantics do not leak inward through the forwarded `object()` call.
- [ ] Tee test: one entry written to a native-object format (EMF) and a flat
      format simultaneously. The native format renders the object; the flat
      format drops it.
- [ ] Warning test: first-write warning fires exactly once per (entry-type,
      format) pair, names the field and format correctly, and does not fire on
      subsequent writes of the same entry type.
- [ ] Array→object→array recursion: a `Vec<Phase>` where `Phase` contains a
      `Vec<SubPhase>` where `SubPhase` contains a `Vec<u64>`. Verify all three
      levels render correctly.
- [ ] Empty arrays: `Each<AsObject>` on an empty `Vec<Phase>` renders `[]`.
- [ ] LocalFormat: verify objects render as native JSON in local output.
- [ ] Compile-fail: a bare struct-typed field without the format attribute.
- [ ] Add crate-level public documentation and a changelog entry.

### Before stabilization

- [ ] Address the boxed entry path (`DynValueWriter` / `ValueWriterFromDyn`):
      either add an `object` bridge to the dyn layer, or document clearly that
      object fields require a non-boxed sink. This is the same architectural gap
      that `values()` has today (stringifies through the dyn boundary).
- [ ] Decide whether duplicate keys within an object should be validated (EMF
      and JSON validate only top-level names today).
- [ ] Decide the empty-object representation (`{}` for present objects; nothing
      for an absent `Option`).
- [ ] Decide the empty-array representation (`Each<AsObject>` on an empty `Vec`
      should likely emit `[]`, matching existing `values()` behavior for empty
      arrays).
- [ ] Decide whether object numeric members should carry sampling multiplicity
      (the recommendation is to ignore it, matching array elements).
- [ ] Review the names `ObjectValue`, `AsObject`, and `Each`.

## Future work

Because rendering is expressed as a formatter that chooses which `ValueWriter`
method to call, the following can be added later without any change to the core
traits proposed here.

- **`AsJson` string formatter.** A `ValueFormatter` for any `T: Serialize` that
  calls `serde_json::to_string(v)` and writes the result through
  `writer.string(...)`. This is a convenience for attaching a structured type as
  a JSON-encoded string without implementing `ObjectValue`. It never produces a
  native object. Gate behind a `serde` feature.
- **Native objects from `serde::Serialize`.** `AsJson` renders a `Serialize`
  type as a JSON _string_. A formatter that renders it as a _native_ object
  would require a `serde::Serializer` implementation that, rather than producing
  bytes, drives `EntryWriter::value(name, v)` and the array machinery. This is a
  well-defined but non-trivial component: it must map serde's data model (struct
  fields, maps, sequences, enums, `Option`) onto object members, reconcile serde
  map keys of arbitrary type against string-only object keys, and decide how
  serde attributes such as `#[serde(flatten)]`, `rename`, and `skip` affect the
  emitted shape. That last point is the reason it is deferred: it makes the
  field's wire shape follow serde's rules rather than metrique's, which is a
  semantic commitment worth making deliberately.
- **A native OTel object representation.** The OTel specification supports
  complex attributes, but the Rust SDK (`opentelemetry` 0.32, the version
  `metrique-otel` depends on) does not yet expose a map/object variant in its
  `Value` enum — it is limited to `Bool | I64 | F64 | String | Array`
  (homogeneous). So native OTel object rendering is blocked on a future SDK
  version adding the complex attribute type. Once available, `metrique-otel`
  would override `object()` to produce the appropriate `Value` variant.
- **Alternative rendering formatters.** An `AsObjectString` formatter (always
  emits a JSON-encoded string, even on formats with native support) and an
  `AsObjectWithStringFallback` formatter (native where supported, string where
  not, rather than drop) can be added if use cases arise. These would use a
  `write_object_as_string` utility function (analogous to
  `write_values_as_string`) that drives the object through a string-building
  `EntryWriter`.
- **Attribute sugar.** Thin sugar such as `#[metrics(object)]` /
  `#[metrics(object_array)]` could desugar to the corresponding `format = As...`
  formatters for readability.
- **`each(F)` parse sugar.** The macro could parse `format = each(F)` as sugar
  for `Each<F>`, saving users from writing the generic syntax directly.
