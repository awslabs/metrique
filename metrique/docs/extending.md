# Extending metrique

Most of the time you'll be using the built-in metric types ([`Timer`] and [`Counter`] from
`metrique`, [`Histogram`] from `metrique-aggregation`, timestamps, plain numbers and
strings) wrapped in a `#[metrics]` struct. But you can extend `metrique` by defining your
own metric types, customizing how a value is formatted, or hand-building an entry.

## The closing model

`metrique` separates a metric's life into two phases:

- **Accumulation** (mutable): while a request is in flight you mutate a struct. Fields like
  [`Timer`], [`Counter`], and [`Histogram`] hold live, changing state.
- **Emission** (immutable): when the work finishes, the struct is _closed_ into a plain,
  inert entry that is handed to a sink.

The one-way transition between them is the [`CloseValue::close`] method. Closing a `Timer`
turns it into the [`Duration`] it measured; closing a `Counter` turns it into the `u64` it
counted; closing a `#[metrics]` struct closes every field and produces an entry.

```rust
use metrique::unit_of_work::metrics;
use metrique::timers::Timer;

#[metrics]
struct RequestMetrics {
    // a live, mutable accumulator during the request...
    operation_time: Timer,
    items: usize,
}
// ...that closes into an inert entry: `operation_time` becomes a `Duration`,
// `items` stays a `usize`. The closed entry is what gets written to the sink.
```

You rarely call `close()` by hand. The guard returned by `append_on_drop(sink)` closes the
struct and appends the entry when it is dropped. The point of the model is _what_ you are
extending: a custom metric type is just a type that knows how to close itself.

## How the traits relate

Four traits, in [`metrique_core`], make up the system. In practice you only ever implement
the first one.

| Trait                | What it does                                                             | You implement it when...                   |
| -------------------- | ------------------------------------------------------------------------ | ------------------------------------------ |
| [`CloseValue`]       | Turns a value into its `Closed` form (the accumulation -> emission step) | Defining a custom leaf or accumulator type |
| [`CloseValueRef`]    | The by-ref version of closing; blanket-implemented from `&T: CloseValue` | Never directly (it is automatic)           |
| [`CloseEntry`]       | Trait alias for `CloseValue<Closed: InflectableEntry>`                   | Never directly (it is an alias)            |
| [`InflectableEntry`] | A renamable, writable metric entry: the closed form of a whole struct    | Only for fully manual entries (rare)       |

The key trait is [`CloseValue`]:

```rust
pub trait CloseValue {
    type Closed;
    fn close(self) -> Self::Closed;
}
```

Two conventions are worth internalizing:

- **Implement it for both `&T` and `T`.** The by-reference impl holds the real logic; the
  by-value impl either proxies to it (via the blanket `.close_ref()` helper) or reads the
  owned value directly when that is cheaper, such as moving a value out instead of cloning it
  or skipping an atomic load. Implementing it for `&T` is what lets the type close without
  giving up ownership, and it is what gives you smart-pointer support (closing an
  `Arc<YourType>`) for free, via the blanket [`CloseValueRef`] impl. You never implement
  [`CloseValueRef`] yourself.
- **`Closed` must be writable.** When your type is used as a metric field, its `Closed` type
  has to implement [`metrique_writer::Value`] so it can actually be written out. The built-in
  scalar types already do.

Closing a whole `#[metrics]` struct produces an [`InflectableEntry`]: an entry whose field
names can still be "inflected" (PascalCase, snake_case, etc.) without any runtime string work.
You almost never touch [`CloseEntry`] or [`InflectableEntry`] directly; the `#[metrics]` macro
generates both for you.

## Recipe: a custom accumulator type

The canonical extension: implement [`CloseValue`] for `&T` and `T`. An accumulator holds
live state that is mutated through `&self` during the request (so it can be shared across
tasks, e.g. behind an `Arc`) and read out on close. That mutation-through-`&self` is exactly
why the by-ref impl carries the logic. Here is a counter backed by an atomic, the same shape
the built-in [`Counter`] uses:

```rust
use metrique::CloseValue;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct ConcurrentCounter(AtomicU64);

impl ConcurrentCounter {
    fn incr(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

// The by-ref impl reads the shared, mutated state. Because it closes through `&self`, the
// counter can be incremented from many tasks (e.g. behind an `Arc`) and still be closed
// without giving up ownership.
impl CloseValue for &'_ ConcurrentCounter {
    type Closed = u64;
    fn close(self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

// The by-value impl can read the owned value directly, avoiding the atomic load.
impl CloseValue for ConcurrentCounter {
    type Closed = u64;
    fn close(self) -> u64 {
        self.0.into_inner()
    }
}
```

`Closed` is `u64`, which already implements [`metrique_writer::Value`], so `ConcurrentCounter`
drops straight into a `#[metrics]` struct as a field. Just like the built-in scalars, a custom
leaf type takes a unit through `#[metrics(unit = ...)]`:

```rust
# use metrique::CloseValue;
# use std::sync::atomic::{AtomicU64, Ordering};
# #[derive(Default)]
# struct ConcurrentCounter(AtomicU64);
# impl CloseValue for &'_ ConcurrentCounter {
#     type Closed = u64;
#     fn close(self) -> u64 { self.0.load(Ordering::Relaxed) }
# }
# impl CloseValue for ConcurrentCounter {
#     type Closed = u64;
#     fn close(self) -> u64 { self.0.into_inner() }
# }
use metrique::unit_of_work::metrics;
use metrique::unit::Count;

#[metrics]
struct MyMetric {
    #[metrics(unit = Count)]
    requests: ConcurrentCounter,
}
```

## Recipe: a custom value wrapper

The accumulator above mutates live state. The other common case is the opposite: a field that
just holds owned data and is closed by moving or cloning it, with no interior mutability. Wrap
the data and implement [`CloseValue`] so its `Closed` type is something writable. Here a
`String` is cloned on the by-ref close and moved out on the by-value one:

```rust
use metrique::unit_of_work::metrics;

struct StringValue(String);

impl metrique::CloseValue for &StringValue {
    type Closed = String;
    fn close(self) -> String {
        self.0.clone()
    }
}

impl metrique::CloseValue for StringValue {
    type Closed = String;
    fn close(self) -> String {
        self.0
    }
}

#[metrics]
struct MyMetric {
    field: StringValue,
}
```

## Recipe: a custom value formatter

When the value is fine but you want to control _how it is rendered_ (without a new type),
implement [`ValueFormatter`] and point a field at it with `#[metrics(format = ...)]`. Here a
[`SystemTime`] is emitted as an RFC 3339 UTC string:

```rust
use metrique::unit_of_work::metrics;
use std::time::SystemTime;
use chrono::{DateTime, Utc};

/// Format a `SystemTime` as a UTC timestamp.
struct AsUtcDate;

// `format_value` is a static method, so `AsUtcDate` is never instantiated.
impl metrique::writer::value::ValueFormatter<SystemTime> for AsUtcDate {
    fn format_value(writer: impl metrique::writer::ValueWriter, value: &SystemTime) {
        let datetime: DateTime<Utc> = (*value).into();
        writer.string(&datetime.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
}

#[metrics]
struct MyMetric {
    #[metrics(format = AsUtcDate)]
    my_field: SystemTime,
}
```

## Recipe (advanced): a manual entry

There is currently no stable, non-macro way to produce an [`InflectableEntry`] that inflects
names. If you need a hand-built entry, implement the [`Entry`] trait directly and pull it in
with `#[metrics(flatten_entry, no_close)]`. Note that this path ignores name inflection.

```rust
use metrique::unit_of_work::metrics;
use metrique::writer::{Entry, EntryWriter};

struct MyCustomEntry;

impl Entry for MyCustomEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("custom", "custom");
    }
}

#[metrics]
struct MyMetric {
    #[metrics(flatten_entry, no_close)]
    field: MyCustomEntry,
}
```

## When to reach for the macro instead

Reach for these recipes for **leaf types** (a new accumulator, a wrapper around an external
type) and for **custom rendering**. For anything that is a _group of fields_, use `#[metrics]`:
it generates the [`CloseValue`] and [`InflectableEntry`] impls, handles name inflection, and
closes each field for you. Hand-implementing the entry traits is the exception, not the rule.

To verify a custom type once you have written it, drop it into a `#[metrics]` struct and use
the helpers in the [testing guide][`testing`] (`test_metric`) to assert on the closed entry.

[`CloseValue`]: https://docs.rs/metrique/latest/metrique/trait.CloseValue.html
[`CloseValue::close`]: https://docs.rs/metrique/latest/metrique/trait.CloseValue.html#tymethod.close
[`CloseValueRef`]: https://docs.rs/metrique/latest/metrique/trait.CloseValueRef.html
[`CloseEntry`]: https://docs.rs/metrique/latest/metrique/trait.CloseEntry.html
[`InflectableEntry`]: https://docs.rs/metrique-core/latest/metrique_core/trait.InflectableEntry.html
[`metrique_core`]: https://docs.rs/metrique-core
[`testing`]: https://docs.rs/metrique/latest/metrique/_guide/testing/
[`Entry`]: https://docs.rs/metrique/latest/metrique/writer/trait.Entry.html
[`ValueFormatter`]: https://docs.rs/metrique/latest/metrique/writer/value/trait.ValueFormatter.html
[`metrique_writer::Value`]: https://docs.rs/metrique/latest/metrique/writer/trait.Value.html
[`Timer`]: https://docs.rs/metrique/latest/metrique/timers/struct.Timer.html
[`Counter`]: https://docs.rs/metrique/latest/metrique/struct.Counter.html
[`Histogram`]: https://docs.rs/metrique-aggregation/latest/metrique_aggregation/struct.Histogram.html
[`Duration`]: https://doc.rust-lang.org/std/time/struct.Duration.html
[`SystemTime`]: https://doc.rust-lang.org/std/time/struct.SystemTime.html
