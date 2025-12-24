This crate contains infrastructure to make writers that emit unit-of-work-based metrics.

If you want to write a library or application that generates unit-of-work
based metrics, the API in this crate is lower-level than you'll want,
and it's normally much easier to do that via the `#[metrics]`
macro in the `metrique` crate, and therefore, the main Getting Started
documentation is in that crate - it is recommended to read that, even
if you end up using just `metrique-writer`.

## Metric Entries

This crate is centered around an [`Entry`] trait. Implementations of
the [`Entry`] trait define a metric that can be emitted in various formats

### Writing Entries

The easiest way to make metric entries is by using the `metrique` crate,
see the docs for the `metrique` crate for that.

It is also possible to make `Entry`es without the `metrique` crate,
by using `#[derive(Entry)]`

```rust
use metrique_writer::{Entry, unit::{AsBytes, AsSeconds}};

use std::time::Duration;

#[derive(Entry)]
pub struct MyInnerEntry {
    inner_value: u32
}

#[derive(Entry)]
pub struct MyEntry {
    property: String,
    value: u32,
    // emitted as 0 or 1
    flag: bool,
    // emitted as milliseconds, as per `impl Value for Duration`
    time: Duration,
    // but you can use AsSeconds etc. to change it to other units
    // to fill it in, use `my_duration.into()`
    time_seconds: AsSeconds<Duration>,
    // you can also use the unit markers to give units to other values
    value_bytes: AsBytes<u32>,
    // if None, not emitted; if Some, emitted as the inner value
    optional_value: Option<AsBytes<u32>>,
    // you can flatten entries
    #[entry(flatten)]
    inner_entry: MyInnerEntry,
}
```

It is also possible to implement `Entry` manually (see the docs for [`Entry`]).

## Emitting Metrics

This library currently only comes with `metrique-writer-format-emf`,
which formats to [Amazon CloudWatch Embedded Metric Format (EMF)][emf-docs],
but more formatters might be added in the future. You can also implement a custom
format using the [`Format`] trait.

Entries are sent to an [`EntrySink`] in order to be written to a destination.

You can either thread the [`EntrySink`] manually in your code, or register
a global entry sink by using the [`sink::global_entry_sink`] macro.

The [`EntrySink`] generally has these 3 components:

1. A [`Format`] that is used to format the metrics to be emitted.
2. Some output file, which turns the [`Format`] to an [`EntryIoStream`]
3. Some queueing policy ([`sink::BackgroundQueue`] or [`sink::FlushImmediately`]).

One example way of setting up metrics would be to do the following:

```rust
use metrique_writer::{
    Entry, GlobalEntrySink,
    format::FormatExt as _,
    sink::{AttachGlobalEntrySinkExt, global_entry_sink},
    unit::AsCount,
};
use metrique_writer_format_emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

global_entry_sink! {
    /// A special metrics sink for my application
    MyEntrySink
}

# let log_dir = tempfile::tempdir().unwrap();

#[derive(Entry, Default)]
struct MyMetrics {
    field: AsCount<usize>,
}

#[derive(Entry)]
struct Globals {
    region: String,
}

let globals = Globals {
    // Generally, this is usually sourced from CLI args or the environment
    region: "us-east-1".to_string(),
};

let join = MyEntrySink::attach_to_stream(Emf::all_validations("MyApp".into(), 
    vec![vec![], vec!["region".into()]] /* emit using the [] and ["region"] dimension sets */)
    // All entries will contain `region: us-east-1` as a property
    .merge_globals(globals)
    .output_to_makewriter(
        RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
    )
);

// you might want to detach the queue - otherwise, dropping the `BackgroundQueueJoinHandle`
// will shut the BackgroundQueue down and then wait for it to be flushed.
// join.forget();

let mut metric = MyEntrySink::append_on_drop_default::<MyMetrics>();
*metric.field += 1;
// metric appends to sink as scope ends and variable drops
```

You can also do it without a global entry sink, if you'll rather not use a global variable:

```rust
use metrique_writer::{
    Entry,
    EntrySink,
    format::FormatExt as _,
    sink::BackgroundQueue,
    unit::AsCount,
};
use metrique_writer_format_emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

# let log_dir = tempfile::tempdir().unwrap();

#[derive(Entry, Default)]
struct MyMetrics {
    field: AsCount<usize>,
}

#[derive(Entry)]
struct Globals {
    region: String,
}


// this will create a BackgroundQueue that works only with the type MyMetrics. This
// is slightly faster, since there is no boxing

let globals = Globals {
    // Generally, this is usually sourced from CLI args or the environment
    region: "us-east-1".to_string(),
};

let (queue, join) = BackgroundQueue::<MyMetrics>::new(Emf::all_validations("MyApp".into(), 
    vec![vec![], vec!["region".into()]] /* emit using the [] and ["region"] dimension sets */)
    // All entries will contain `region: us-east-1` as a property
    .merge_globals(globals)
    .output_to_makewriter(
        RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
    )
);

// you might want to detach the queue - otherwise, dropping the `BackgroundQueueJoinHandle`
// will shut the BackgroundQueue down and then wait for it to be flushed.
// join.forget();

let mut metric = queue.append_on_drop_default();
*metric.field += 1;
// metric appends to sink as scope ends and variable drops
```

Or without a global entry sink, but still with a queue that accepts multiple metric types:

```rust
use metrique_writer::{
    Entry,
    BoxEntry,
    AnyEntrySink,
    format::FormatExt as _,
    sink::BackgroundQueueBuilder,
    unit::AsCount,
};
use metrique_writer_format_emf::Emf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

# let log_dir = tempfile::tempdir().unwrap();

#[derive(Entry, Default)]
struct MyMetrics {
    field: AsCount<usize>,
}

#[derive(Entry)]
struct Globals {
    region: String,
}


// this will create a BackgroundQueue that works only with the type MyMetrics. This
// is slightly faster, since there is no boxing

let globals = Globals {
    // Generally, this is usually sourced from CLI args or the environment
    region: "us-east-1".to_string(),
};

let (queue, join) = BackgroundQueueBuilder::new().build_boxed(
    Emf::all_validations("MyApp".into(), 
        vec![vec![], vec!["region".into()]] /* emit using the [] and ["region"] dimension sets */)
        // All entries will contain `region: us-east-1` as a property
        .merge_globals(globals)
        .output_to_makewriter(
            RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
        )
);

// you might want to detach the queue - otherwise, dropping the `BackgroundQueueJoinHandle`
// will shut the BackgroundQueue down and then wait for it to be flushed.
// join.forget();

let mut metric = MyMetrics::default();
*metric.field += 1;
queue.append_any(metric);
```

[emf-docs]: htps://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html
[`Format`]: https://docs.rs/metrique/0.1/metrique/writer/format/trait.Format.html
