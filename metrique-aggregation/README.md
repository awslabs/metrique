When emitting high-frequency metrics, you often want to aggregate multiple observations into a single metric entry rather than emitting each one individually. This crate provides an aggregation system for metrique that collects observations and emits them as distributions, sums, or other aggregate forms.

# When to Use Aggregation

Consider aggregation when:

- **High-frequency, low-level events**: TLS handshakes, storage operations, or other infrastructure-level metrics
- **Fan-out operations**: A single unit of work spawns multiple sub-operations you want to aggregate
- **Background processing**: Queue workers that generate one metric per processed item at an extremely high rate

Sampling raw records is often a better approach than aggregation (and should often be combined with aggregation!) Preserving raw records can make it much easier to debug issues.

The power of metrique and metrique aggregation allow you do to both:
- Emit a sampled set of raw events (e.g. with congressional sampling to ensure all errors are preserved) to one sink 
- Emit aggregated events to a second sink

This can let you get the best of both worlds. For more info on this pattern, see the `split` example.

# Examples

This crate includes several complete examples:

- `embedded` - Distributed query with [`Aggregate<T>`]
- `sink_level` - Queue processor with [`WorkerSink`] and [`KeyedAggregator`]
- `split` - [`SplitSink`] pattern showing aggregation + raw events
- `histogram` - Histogram usage patterns and strategies

Run examples with: `cargo run --example <name>`

# Quick Start

## Embedded Aggregation

Use the [`aggregate`] macro to mark an `#[metrics]` struct as aggregatable. You will need to define
strategies for each field to describe how multiple items will be merged.

```rust,no_run
use metrique::{unit_of_work::metrics, ServiceMetrics, unit::Millisecond};
use metrique::writer::GlobalEntrySink;
use metrique_aggregation::{aggregate, histogram::Histogram, value::Sum};
use metrique_aggregation::aggregator::Aggregate;
use std::time::Duration;

#[aggregate]
#[metrics]
struct ApiCall {
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
    
    #[aggregate(strategy = Sum)]
    response_size: usize,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    request_id: String,
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCall>,
}

# fn main() {
let mut metrics = RequestMetrics {
    request_id: "query-123".to_string(),
    api_calls: Aggregate::default(),
}.append_on_drop(ServiceMetrics::sink());

// Add multiple observations
metrics.api_calls.insert(ApiCall {
    latency: Duration::from_millis(45),
    response_size: 1024,
});
metrics.api_calls.insert(ApiCall {
    latency: Duration::from_millis(67),
    response_size: 2048,
});

// When metrics drops, emits a single entry with aggregated values
# }
```

**Output**: Single metric entry with `RequestId: "query-123"`, `Latency: [45ms, 67ms]`, `ResponseSize: 3072`

## Sink-Level Aggregation

Use [`WorkerSink`] or [`MutexSink`] when you want to produce aggregated metric entries where the entire entry is aggregated. Both can be combined with [`KeyedAggregator`] to perform aggregation against a set of keys or [`Aggregate`] when there are no keys. These sinks will be backed by a traditional sink that emits to EMF or other destination.

[`WorkerSink`] performs aggregation in a background thread that periodically flushes aggregated data to a backing sink. [`MutexSink`] is alternative sink that manages concurrency with a mutex instead of a channel.

```rust
use metrique::unit_of_work::metrics;
use metrique::timers::Timer;
use metrique_aggregation::sink::DropGuard;
use metrique_aggregation::{aggregate, histogram::Histogram, value::Sum};
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::sink::WorkerSink;
use std::time::Duration;

#[aggregate]
#[metrics]
struct QueueItem {
    #[aggregate(key)]
    item_type: String,
    
    #[aggregate(key)]
    priority: u8,
    
    #[aggregate(strategy = Sum)]
    items_processed: u64,
    
    #[aggregate(strategy = Histogram<Duration>)]
    processing_time: Timer,
}

async fn process_item(_item: &str, mut entry: impl DropGuard<QueueItem>) {
    // when `entry` is dropped, it will be added to the sink.
    // the timer will stop when it is dropped.
    entry.items_processed += 1;
}
# struct Item { type_name: String, priority: u8 }
# async fn get_item() -> Option<Item> { None }
async fn setup_queue_processor() {
    # use metrique::test_util::test_entry_sink;
    # let base_sink = test_entry_sink().sink;
    let keyed_aggregator = KeyedAggregator::<QueueItem>::new(base_sink);
    let sink = WorkerSink::new(keyed_aggregator, Duration::from_secs(60));
    
    // Process queue items
    while let Some(item) = get_item().await {
        let metrics = QueueItem {
            item_type: item.type_name.clone(),
            priority: item.priority,
            items_processed: 1,
            processing_time: Timer::start_now(),
        }.close_and_merge(sink.clone());
        process_item(&item.type_name, metrics).await;
    }
    
    // Periodically flushes aggregated results (every 60 seconds)
}
# fn main() {}
```

**Output**: Multiple aggregated entries like `ItemType: "email", Priority: 1, ItemsProcessed: 1247, ProcessingTime: [histogram]`

**Choosing between WorkerSink and MutexSink:**

- **[`MutexSink`]** - Use when you have inputs from a smaller number of threads. Great for supporting `close_and_merge` with embedded metrics. Currently does not support automatic flushing.
- **[`WorkerSink`]** - Use for sink-level aggregation from many producers across many threads. The channel-based design reduces contention and provides configurable flush timing.

See the `sink_level` example for a complete working implementation.

# Core Concepts

## Field-Level Strategies

Individual fields use aggregation strategies that implement [`AggregateValue<T>`]:

- **[`Sum`]** - Sums values together (use for counts, totals)
- **[`Histogram<T>`]** - Collects values into a distribution (use for latency, sizes)
- **[`KeepLast`]** - Keeps the most recent value (use for gauges, current state)

## Entry-Level Aggregation

The [`aggregate`] macro generates implementations that define how complete entries are combined. It creates the merge logic, key extraction, and aggregation strategy for your type.

## Keys

Fields marked with `#[aggregate(key)]` become grouping keys. Entries with the same key are merged together when using a
[`KeyedAggregator`].

```rust
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, histogram::Histogram};
use std::time::Duration;

#[aggregate]
#[metrics]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,
    
    #[aggregate(strategy = Histogram<Duration>)]
    latency: Duration,
}
# fn main() {}
```

Calls to the same endpoint will be aggregated together, while different endpoints remain separate.

## Aggregation Traits and How They Work Together

The aggregation system is built on several traits that work together:

- **[`AggregateValue<T>`]** - Defines how individual field values are merged (Sum, Histogram, KeepLast)
- **[`Merge`]** - Defines how complete entries are merged together by consuming the source
- **[`MergeRef`]** - Like [`Merge`], but merges by reference (enables [`SplitSink`] to send to multiple destinations)
- **[`Key`]** - Extracts grouping keys from entries to determine which entries should be merged
- **[`AggregateStrategy`]** - Ties together the source type, merge behavior, and key extraction
- **[`AggregateSink<T>`]** - Destination that accepts and aggregates entries

The [`aggregate`] macro generates implementations of these traits for your type. For most use cases, you don't need to implement these manually - the macro handles it.

For more detail, see the [`traits`] module.


# Advanced Usage Patterns

## Split Aggregation

When you use `#[aggregate(ref)]`, it makes it possible to send the same record to multiple different sinks. This allows
aggregation by different sets of keys as well as sending the raw, unaggregated record directly to a sink.

You can use [`SplitSink`] to aggregate the same data to multiple destinations - useful for combining precise aggregated metrics with sampled raw events. Split aggregation can also allow aggregating the same metric by multiple different sets of dimensions (see the `split` example).

```rust
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::sink::{SplitSink, EntrySinkAsAggregateSink, WorkerSink};
# use metrique::unit_of_work::metrics;
# use metrique_aggregation::{aggregate, histogram::Histogram};
# use std::time::Duration;
// to use multi-sink aggregation, it must be possible to aggregate by reference:
#[aggregate(ref)]
#[metrics]
struct QueueItem {
    #[aggregate(key)]
    item_type: String,
    #[aggregate(strategy = Histogram<Duration>)]
    processing_time: Duration,
}

# fn main() {
# use metrique::test_util::test_entry_sink;
# let aggregated_sink = test_entry_sink().sink;
# let raw_events_sink = test_entry_sink().sink;
// Aggregator for precise counts
let aggregator = KeyedAggregator::<QueueItem>::new(aggregated_sink);

// Raw sink for sampling individual events
let raw = EntrySinkAsAggregateSink::new(raw_events_sink);

// Combine them
let split = SplitSink::new(aggregator, raw);
let sink = WorkerSink::new(split, Duration::from_secs(60));

// Each entry goes to both sinks
QueueItem {
    item_type: "email".to_string(),
    processing_time: Duration::from_millis(10),
}
.close_and_merge(sink.clone());
# }
```


This gives you:
- **Precise aggregated metrics**: Exact counts and distributions
- **Raw event samples**: Individual events for tracing and debugging

See the `split` example for a complete working implementation.

# Histograms

When aggregating data, a Histogram is often the best way to do it. When you flatten state down into a "gauge" field, such as with `KeepLast`, you often lose critical information, but a histogram can capture a much richer picture. Histograms collect observations into distributions, allowing you to track percentiles, min, max, and other statistical properties. Histograms can be used with `#[aggregate]` or embedded directly in your metrics.

## Basic Usage

```rust
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::Histogram;
use metrique_writer::unit::Millisecond;
use std::time::Duration;

#[metrics]
struct Metrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration>,
}

# fn main() {
let mut metrics = Metrics {
    latency: Histogram::default(),
};

metrics.latency.add_value(Duration::from_millis(10));
metrics.latency.add_value(Duration::from_millis(20));
metrics.latency.add_value(Duration::from_millis(15));

// write `metrics` to a sink to persist it...
# }
```

## Aggregation Strategies

Histograms support different bucketing strategies:

- **[`ExponentialAggregationStrategy`]** (default) - Exponential bucketing with ~6.25% error, memory efficient
- **[`SortAndMerge`]** - Stores all observations exactly for perfect precision
- **[`AtomicExponentialAggregationStrategy`]** - Thread-safe exponential bucketing for [`SharedHistogram`]

```rust
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use std::time::Duration;

# fn example() {
// Use SortAndMerge for exact precision
let histogram: Histogram<Duration, SortAndMerge<64>> = Histogram::default();
# }
# fn main() {}
```

## Thread-Safe Histograms

For concurrent access, use [`SharedHistogram`]:

```rust
use metrique_aggregation::histogram::SharedHistogram;
use std::sync::Arc;

# fn main() {
let histogram = Arc::new(SharedHistogram::<u64>::default());

// Can be shared across threads
let h = histogram.clone();
std::thread::spawn(move || {
    h.add_value(42);
});
# }
```

See the `histogram` example for more usage patterns.



[`aggregate`]: macro@crate::aggregate
[`AggregateValue<T>`]: crate::traits::AggregateValue
[`Sum`]: crate::value::Sum
[`Histogram<T>`]: crate::histogram::Histogram
[`KeepLast`]: crate::value::KeepLast
[`Aggregate<T>`]: crate::aggregator::Aggregate
[`WorkerSink`]: crate::sink::WorkerSink
[`MutexSink`]: crate::sink::MutexSink
[`RootSink`]: crate::traits::RootSink
[`KeyedAggregator`]: crate::aggregator::KeyedAggregator
[`SplitSink`]: crate::sink::SplitSink
[`EntrySinkAsAggregateSink`]: crate::sink::EntrySinkAsAggregateSink
[`Merge`]: crate::traits::Merge
[`MergeRef`]: crate::traits::MergeRef
[`Key`]: crate::traits::Key
[`AggregateStrategy`]: crate::traits::AggregateStrategy
[`AggregateSink<T>`]: crate::traits::AggregateSink
[`ExponentialAggregationStrategy`]: crate::histogram::ExponentialAggregationStrategy
[`SortAndMerge`]: crate::histogram::SortAndMerge
[`AtomicExponentialAggregationStrategy`]: crate::histogram::AtomicExponentialAggregationStrategy
[`SharedHistogram`]: crate::histogram::SharedHistogram
