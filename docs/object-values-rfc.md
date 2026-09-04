# RFC: Object-valued fields

> Status: RFC (design with prototype)
>
> Applies to: `metrique-writer-core`, `metrique-writer`,
> `metrique-writer-format-emf`, `metrique-writer-format-json`, `metrique-macro`

This RFC proposes a way to emit a struct field as a nested object rather than a
flat scalar. On formats that support structured values (JSON, EMF), the field is
written as a native nested object. On formats that do not, the field falls back
to a JSON-encoded string (matching the existing `values()` precedent). A
descriptor-time warning fires once at setup to inform the user which fields are
being serialized rather than rendered natively. This addresses
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

_This RFC is deliberately scoped to the object-rendering primitive and its
derive integration. Some ergonomic concerns are deferred for follow up work._

_The design has been prototyped end to end, and the claims below about what
compiles, what allocates, and what the output looks like come from that
prototype rather than from reasoning. I have flagged the places where I am
inferring instead. The prototype is not proposed as the implementation; it is
how the design was tested. Anything described as observed there should be
re-confirmed as it lands._

---

## Motivation

`metrique` maps each field of a `#[metrics]` struct to either a metric (a number
or distribution with a unit and dimensions) or a property (a string, an
identifier, some context). Fields can carry repeated observations (histograms,
distributions), but there is currently no way to attach a structured, nested
object with user-defined subfields to an entry.

The concrete case that motivates this proposal is a tree of units of work whose
shape is not known at compile time. A request is a unit of work, and so is each
phase within it. Consider a calculator service that evaluates an expression such
as `(2 + 4) * (8 - 1)`: the request has a top-level unit of work, and each phase
of evaluation (parsing, each operation) is itself a unit of work. The phases
nest, and the set of operations is defined by the caller's input rather than by
the service author.

The phase tree is not known at compile time, because phases can be created by
customers. At scale we cannot emit a full record per phase, so phases are
aggregated. However, we also want to sample some requests with their phase trees
intact, so that a sampled request and its phases can be correlated. The
structure _is_ the data here: parent/child nesting cannot be recovered from
correlated IDs without emitting explicit parent pointers and rebuilding the tree
at query time. The desired output attaches the phase tree to the request entry
as a native nested structure:

```json
{
  "RequestId": "abcd1234",
  "Phases": [
    { "Kind": "parse_expr", "DurationMs": 11 },
    {
      "Kind": "eval",
      "DurationMs": 22,
      "Children": [
        {
          "Kind": "multiply",
          "DurationMs": 33,
          "Children": [
            { "Kind": "add", "DurationMs": 44 },
            { "Kind": "sub", "DurationMs": 55 }
          ]
        }
      ]
    }
  ]
}
```

More generally, structured context (an error detail, a request shape, a list of
downstream calls) is currently either flattened into the metric namespace, which
changes what the fields mean, or hand-serialised to a string, which throws away
structure that EMF and JSON could carry.

## Non-goals

- Objects are not metrics. Units, dimensions, and flags do not apply inside an
  object, and nested object fields cannot participate in `emf::dimension_sets`.
- Nested object fields do not become aggregate metrics in a parent aggregate
  record. Aggregate them separately with a normal strategy, and mark the object
  field `#[aggregate(ignore)]`.
- No structural descriptor for object internals. See "Descriptors" for why this
  is the right answer rather than a shortcut.
- `Vec<Vec<T>>` is deferred rather than designed for. A tree is `Vec<Phase>`
  where `Phase` contains `Vec<Phase>`, never a nested `Vec`, so nothing here
  needs it. It could be added later if a real use case appears (see open
  question 3).
- Sharing a single finalized child across more than one record is out of scope
  for this RFC. The hand-written `Arc` pattern works today (see the `no_close`
  note under "Supported field shapes"); the ergonomic story is follow-up work.

---

# User-facing documentation

This section is written as the public documentation users should be able to read
once the feature is released.

## Choosing how to attach a child

Consider the following questions:

**Is it one child whose fields are metrics of the parent?** Use `flatten`. A
latency and a retry count belong in the parent's namespace, and `flatten` keeps
their units and dimensions.

**Is it many children whose per-child structure, order, or other individual
detail matters?** Use `AsObject`. The sub-operation tree in #355 is the case:
which phase nested inside which is the data, so collapsing them loses the point.

**Is it many children whose per-child structure does _not_ matter?** Aggregate
them. `Aggregate<T>` merges N children field-by-field, each field per its
strategy (`Histogram` produces a distribution, `Sum` a total, `KeepLast` a
gauge), and flattens the merged result into the parent, so a set of `Phase`
values becomes a flat `DurationMs: {histogram}` rather than an array of objects.
This is the README's first example and works today.

The distinction between the last two is worth being explicit about, because
reaching for `AsObject` when you wanted aggregation is an easy mistake and
produces a much larger record. Objects preserve per-element structure;
aggregation discards it and keeps a merged value per field. Constructing an
`Aggregate<T>` directly from a `Vec<Phase>` you already hold is a separate
ergonomic gap tracked in #386.

```rust
#[metrics(rename_all = "PascalCase")]
struct Timing {
    #[metrics(unit = Millisecond)]
    elapsed: Duration,
    retries: u32,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    /// `Elapsed` and `Retries` become metrics of the request, with units preserved.
    #[metrics(flatten)]
    timing: Timing,

    /// A nested object. Its members are properties, so `Elapsed` carries no unit.
    #[metrics(format = AsObject)]
    shape: Timing,
}
```

Nothing about `Timing` is object-specific. The same struct can be flattened,
nested as an object, or emitted on its own.

## Supported field shapes

`AsObject` works on a `#[metrics]` type and on the common wrappers around one:

| Field type                | Emits                                         |
| ------------------------- | --------------------------------------------- |
| `Phase`                   | one object                                    |
| `Vec<Phase>`              | array of objects                              |
| `Option<Phase>`           | the object, or the key is omitted when `None` |
| `Box<Phase>`              | one object                                    |
| `Arc<Phase>`              | one object (see the `no_close` note below)    |
| `Option<Vec<Arc<Phase>>>` | array of objects, or omitted                  |

Recursion works, so a type may contain a `Vec` of itself. The object type may be
a `#[metrics]` struct or an entry-mode enum; an enum renders as an object the
same way it renders as a flat entry (see "Generated `ObjectValue` impls").

An empty `Vec` emits `[]` and an object whose members all write nothing emits
`{}`. Neither is omitted, so a consumer can distinguish "no children" from
"children not recorded".

A field that closes normally (the common case) needs no extra attribute: a plain
`phases: Vec<Phase>` closes each `Phase` and renders the result as an array of
objects.

A field that already holds _closed_ entries (`phases: Vec<PhaseEntry>`,
where `PhaseEntry` is declared `#[metrics(closeable_entry)]`) also needs no
`#[metrics(no_close)]`, because `closeable_entry` gives the closed type an
identity `CloseValue` impl.

The one shape that still needs `#[metrics(no_close)]` is an `Arc` of an
already-closed entry:

```rust
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(format = AsObject, no_close)]
    phases: Vec<Arc<PhaseEntry>>,
}
```

An `Arc` cannot be consumed, so it can only close _by reference_, which requires
`CloseValueRef`—`&PhaseEntry: CloseValue`, producing an owned closed entry
from a borrow. `closeable_entry` provides only the by-value identity impl, and a
by-reference one cannot be generated in general: it would have to clone, but a
closed entry is not necessarily `Clone` (neither a histogram's nor a timer's
closed form is), and even where it is, cloning through the `Arc` would discard
the sharing. So the derive cannot close an `Arc<PhaseEntry>` on its own;
`no_close` tells it the field already holds a finalized value and should be
rendered as-is. Close first, then wrap: `Arc<T>: CloseValue` closes to
`T::Closed`, not `Arc<T::Closed>`, so closing _through_ an `Arc` would discard
the sharing. This can be addressed in follow up work.

## What formats do

| Format        | Object                                         | Array of objects              |
| ------------- | ---------------------------------------------- | ----------------------------- |
| EMF           | native nested object in the properties section | native array                  |
| JSON          | native nested object                           | native array                  |
| `LocalFormat` | native                                         | native                        |
| OTel*         | JSON-encoded string attribute                  | one JSON-encoded array string |
| Anything else | JSON-encoded string                            | JSON-encoded array string     |

The fallback is a string rather than a drop, so a field is never silently lost.
The array fallback is bracketed, so one `JSON.parse` recovers it.

_\* NOTE: OTel supports object properties as
[Complex Attributes](https://opentelemetry.io/blog/2025/complex-attribute-types/),
but
[opentelemetry-sdk](https://docs.rs/opentelemetry_sdk/0.32.1/opentelemetry_sdk/)
has not been updated to reflect that yet._

## Naming

A nested object is a **fresh naming scope**. The child's own `rename_all`
applies; if it declares none, its members are emitted verbatim. The parent's
style does not flow across the object boundary.

This differs from `flatten`, where the parent's case style does propagate, and
the difference is deliberate. Under transitive naming a `Phase` emitted
standalone has `duration_ms` while the same `Phase` inside a `PascalCase` parent
has `DurationMs`: same type, same data, two schemas, and anything consuming both
has to know which context each record came from. Emitting a sampled embedded
child and the same child aggregated separately is the entire point of #355, so a
stable per-type schema matters more than one uniform convention per document.

The fresh scope also gives the right answer for `prefix`. A parent's prefix
belongs on the object's own key, not on its members, and a fresh scope produces
that without a special case.

The **default** is the one naming decision that must be settled before this
ships, because it is wire-observable: changing it later changes emitted field
names, and every dashboard, alarm, and saved query built on them stops matching,
even though all downstream code still compiles. A field-level override is
additive and can be tracked as its own issue.

---

# Reference

## `ObjectValue`

```rust
pub trait ObjectValue {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>);
}
```

The object writer is `EntryWriter`, reusing the existing named-field visitor
rather than introducing a parallel one. An entry and an object are both "a
sequence of named values"; `HashMap<K, V>: Entry` already demonstrates that.

Two things make this safe rather than sloppy. `EntryWriter::timestamp` and
`::config` are meaningless inside an object body and are **silently ignored**;
in the prototype, an object body calling both left the enclosing entry's
timestamp and dimension sets untouched. And the member writer strips unit,
dimension, and flag metadata, so no `_aws` metadata can reach a nested object.
Object internals are properties structurally, not by convention.

`ObjectValue` is not dyn-compatible, because `write_object` is generic over the
writer. That is not a regression: the boxed-entry path already bridges it
through an internal `DynObjectValue`.

## `ValueWriter::object`

```rust
fn object<O: ObjectValue + ?Sized>(self, object: &O) {
    write_object_as_string(self, object)
}
```

The default is a JSON-encoded string. A no-op default was the earlier proposal,
and prototyping it showed why that is the wrong choice: with a no-op default,
EMF dropped the field entirely and `Emf::all_validations` still returned `Ok`.
Unlike `values`, which at least degrades to something visible in the output, a
dropped object leaves no evidence.

`object` should be added to the `metrique_require_explicit_impls` gate (#336) so
first-party writers state their choice.

## `AsObject` and the wrapper matrix

`AsObject` is a `ValueFormatter`. It needs three impls, all `NotLifted`:

```rust
impl<O: ObjectValue + ?Sized> ValueFormatter<O, NotLifted> for AsObject { … }
impl<O: ObjectValue> ValueFormatter<Vec<O>, NotLifted> for AsObject { … }
impl<O: ObjectValue> ValueFormatter<Option<O>, NotLifted> for AsObject { … }
```

`Arc<O>` and `Box<O>` need no formatter impls at all, because `ObjectValue`
itself forwards through them:

```rust
impl<O: ObjectValue + ?Sized> ObjectValue for Box<O> { … }
impl<O: ObjectValue + ?Sized> ObjectValue for Arc<O> { … }
```

That is why the wrapper matrix falls out without per-shape fights, and why
`AsObject` does not need `Lifted` liftability. `Option<Vec<O>>` needs one more
impl of the same shape.

Note that the scalar and `Vec` impls are disjoint only as long as no `Vec<_>` is
ever `ObjectValue`. That holds today, and both impls should carry a comment
saying so.

## Generated `ObjectValue` impls

The derive generates, per entry type:

```rust
impl ObjectValue for FooEntry {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        <Self as InflectableEntry<Identity>>::write(self, writer)
    }
}
```

Per-type generation is the only viable route, and this is worth recording
because both blanket alternatives look plausible and neither works:

- `impl<E: Entry> ObjectValue for E` compiles and **never fires**. In the
  prototype it produced `error[E0277]: FooEntry is not a metric entry` at every
  use site, because generated closed structs implement `InflectableEntry`, not
  `Entry`. The only bridge to `Entry` is
  `impl<M: InflectableEntry> Entry for RootEntry<M>`.
- `impl<E: InflectableEntry> ObjectValue for E` is an `E0210` orphan violation
  from `metrique-core`, since `ObjectValue` lives in `metrique-writer-core` and
  `metrique-core` depends on it.

The general lesson, which recurred three times while prototyping this: coherence
only ever grants you a type you own. Every dead end here was an attempt to
express a relationship between two foreign types, and every fix introduced a
local one.

If #166 lands and moves the `NameStyle` parameter to `CloseValue`,
`InflectableEntry` disappears and the first blanket becomes viable. This design
does not depend on that either way.

The same impl is generated for entry-mode enums, not just structs. An entry enum
already implements `InflectableEntry` (that is how it renders when flattened),
so the identical one-line delegation makes it render as an object exactly the
way it renders as a flat entry today: the active variant's fields, untagged by
default, with a variant-name tag member when `#[metrics(tag(...))]` is set. The
entry stays representation-neutral. Whether a nested enum object should instead
be _externally_ tagged (`{"Read": {…}}`, fields nested under the variant name)
is a formatter or writer decision layered on top, not something baked into the
entry, so it can be added later without changing what the derive emits.

## Arrays

`AsObject`'s `Vec` impl reinterprets the slice rather than materialising
wrappers:

```rust
#[repr(transparent)]
pub struct ObjectRef<O: ?Sized>(O);

impl<O: ObjectValue + ?Sized> Value for ObjectRef<O> {
    const SHAPE: FieldShape<'static> = FieldShape::Object;
    fn write(&self, writer: impl ValueWriter) { writer.object(&self.0) }
}
```

`&[O]` becomes `&[ObjectRef<O>]` in place, so `writer.values(…)` sees elements
that implement `Value` without a collect. This is a pointer cast, and it is why
an `Each<F>` combinator is unnecessary. An earlier draft had one; it allocated
once per array per nesting level, and for a recursive tree that compounds.

The cast is the one piece of `unsafe` in the design. It should live in a single
function with the `#[repr(transparent)]` invariant documented on it.

## The string fallback for arrays

`write_values_as_string` comma-joins elements. For objects that produces
`{…},{…}` with no brackets, which no consumer can parse. The fallback therefore
branches on the element shape:

```rust
let bracket = matches!(V::SHAPE, FieldShape::Object);
```

This is a compile-time branch on an associated const, so non-object lists are
unaffected.

The shape has to be carried across the boxed-entry boundary as well, because
`ValueFromDyn` erases its element's `SHAPE` to `Opaque`. `Value::SHAPE` is a
const, so the wrapper needs a type-level shape marker and `values_dyn` needs to
receive the element shape. #390 restores per-element _typing_ across this
boundary (each element crosses as `&dyn DynValue`), but not the element's
`SHAPE`, so carrying the shape is part of this feature's work rather than
something #390 delivers. The match over `FieldShape` there should be exhaustive
rather than using a wildcard: the enum is `#[non_exhaustive]` but local to the
crate, so a new shape that needs its own treatment then fails to compile instead
of silently taking the opaque path.

## The boxed-entry bridge

`DefaultSink` is `BoxEntrySink`, so this path is the common one and it must not
be left for later. The prototype builds it with `EntryWriter` as the object
writer, which settles an earlier concern that doing so would force a narrower
`ObjectWriter` trait. The bridge mirrors the existing `DynEntry`/`DynValue`
pattern:

```rust
trait DynObjectValue {
    fn write_object<'a>(&'a self, writer: &mut dyn DynEntryWriter<'a>);
}
```

Lifetime-generic methods are dyn-compatible; only type and const generics are
not. `DynEntry::write` already has this exact shape.

## Descriptors

Add `FieldShape::Object`. Adding a variant is additive under
`#[non_exhaustive]`, and the descriptor docs already sanction new variants.

`Object` is a **leaf** marker. It does not describe the object's members, and
that is correct rather than a shortcut: a recursive type has no finite
structural descriptor. `ShapeRef` cannot reference an `EntryDescriptor` anyway.

Object fields carry their flags and appear in the parent's descriptor like any
other field, so a descriptor-aware sink can gate an entire nested subtree by
flag using the mechanism `metrique/tests/descriptor_sink.rs` already exercises.
Members inside an object cannot be filtered individually, which is the
appropriate granularity for gating verbose context.

## Memory Implications

Measured on the prototype with a counting global allocator, on the render step
only, constructing everything before resetting the counter, in debug and
release:

| Shape                                     | Allocations |
| ----------------------------------------- | ----------- |
| single object                             | 0           |
| array of 1, 8, 9, 64 objects              | 0           |
| 31-node and 91-node trees                 | 0           |
| `Arc`-shared children, shared-subtree DAG | 0           |

Identical under EMF and JSON. A `SmallVec` spill at N=64 would have shown up and
did not, which is the evidence that `ObjectRef::wrap_slice` is a cast rather
than a collect.

The boxed path adds 3 allocations per `values` call that exceeds
`VALUES_INLINE_CAPACITY` (prototype uses 8, but could be adjusted).

---

# Alternatives considered

These are the designs that were tried and rejected on the way to the one above.

- **A new value kind with a dedicated `ObjectWriter` trait.** An earlier design
  modeled a nested object as its own kind of `Value` written through a bespoke
  `ObjectWriter`. Rejected because an object is "a sequence of named values,"
  which is exactly the existing `EntryWriter` contract (`HashMap<K, V>: Entry`
  already demonstrates it). Reusing `EntryWriter` avoids a parallel visitor and
  lets any `#[metrics]` struct nest with no new machinery. See
  "[`ObjectValue`](#objectvalue)".
- **A dedicated `object_array` (or entry-accepting array) method on
  `ValueWriter`.** Rejected: every format and every wrapper would need to
  implement a new method. Instead each element is bridged to `Value` through a
  `#[repr(transparent)]` `ObjectRef` cast, so the existing `values()` path
  renders arrays with no new writer method. See "[Arrays](#arrays)".
- **An `Each<F>` array combinator.** An early draft materialised array wrappers
  through an `Each<F>` combinator. Rejected: it allocated once per array per
  nesting level, which compounds for a recursive tree. The slice cast is
  allocation-free (see "[Memory Implications](#memory-implications)").
- **Blanket `ObjectValue` impls.** `impl<E: Entry> ObjectValue for E` compiles
  but never fires, because generated closed structs implement
  `InflectableEntry`, not `Entry`; `impl<E: InflectableEntry> ObjectValue for E`
  is an orphan violation from `metrique-core`. Rejected in favor of per-type
  derive generation. See
  "[Generated `ObjectValue` impls](#generated-objectvalue-impls)".
- **A no-op default for `ValueWriter::object`.** Rejected because on a format
  without native object support it dropped the field silently, leaving no
  evidence in the output and still returning `Ok` from validation. The default
  is a JSON-encoded string instead. See
  "[`ValueWriter::object`](#valuewriterobject)".
- **Transitive naming across the object boundary.** Rejected because letting the
  parent's `rename_all` flow into the object would make the same type emit two
  different schemas depending on the parent it is nested in. A nested object is
  a fresh naming scope. See "[Naming](#naming)".
