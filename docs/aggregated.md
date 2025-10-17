# Metrique Aggregation

**Aggregation is an optional optimization for specific high-volume scenarios. For most applications, [sampling](sampling.md) is the better approach.**

Metrique's aggregation system allows multiple metric entries to be combined in-memory before emission, reducing backend load while preserving statistical accuracy. This is valuable when sampling alone doesn't meet your requirements.

## When to Use Aggregation

Consider aggregation when:

- **High-frequency, low-level events**: TLS handshakes, storage operations, or other infrastructure-level metrics
- **Background processing**: Queue workers or embedded processors that generate one metric per request
- **Fan-out operations**: Tasks that spawn multiple sub-operations you want to aggregate

**Most request/response services should use sampling instead of aggregation.**

## Best of Both Worlds

Aggregation works alongside sampling - you can emit aggregated metrics for precise counts while sampling a fraction of raw events for traceability and debugging.

## Two Usage Patterns

### 1. Embedded Aggregation (`Aggregated<T>`)

Use when a single unit of work fans out to multiple sub-operations:

```rust
use metrique::unit_of_work::metrics;
use metrique::Aggregated;
use metrique::writer::merge::{Counter, Histogram};
use std::time::Duration;
use uuid::Uuid;

// Main task that fans out to backend services
#[metrics]
struct DistributedQuery {
    query_id: String,

    #[metrics(flatten)]
    backend_calls: Aggregated<BackendCall>,
}

// Keyless aggregation - all backend calls merge together
#[metrics(aggregate)]
struct BackendCall {
    #[metrics(aggregate = Counter)]
    requests_made: u64,

    #[metrics(aggregate = Histogram)]
    latency: Duration,

    #[metrics(aggregate = Counter)]
    errors: u64,
}

async fn execute_distributed_query(query: &str) {
    let mut metrics = DistributedQuery {
        query_id: Uuid::new_v4().to_string(),
        backend_calls: Aggregated::new(),
    }.append_on_drop(ServiceMetrics::sink());

    // Fan out to 5 backend services
    for backend in &["shard1", "shard2", "shard3", "shard4", "shard5"] {
        // Create a backend call that auto-aggregates on drop
        let backend_call = metrics.backend_calls.append_on_drop();

        let result = Instrumented::instrument_async(backend_call, async |call_metrics| {
            call_metrics.requests_made = 1;
            let start = Instant::now();
            let result = call_backend(backend, query).await;
            call_metrics.latency = start.elapsed();
            result
        })
        .await
        .finalize_metrics(|call_result, call_metrics| {
            if call_result.is_err() {
                call_metrics.errors = 1;
            }
        })
        // drops the metric, emitting it to the sink
        .emit();
    }

    // Metrics automatically emitted when dropped
}
```

**Output**: Single metric entry with `QueryId: "550e8400-e29b-41d4-a716-446655440000"`, `RequestsMade: 5`, `Latency: [45ms, 67ms, 52ms, 71ms, 58ms]`, `Errors: 1`

### 2. Sink-Level Aggregation (`AggregatingEntrySink`)

Use for extremely high-rate events where you want "true unsampled metrics":

```rust
use metrique::unit_of_work::metrics;
use metrique::writer::merge::{AggregatingEntrySink, Counter, Histogram};
use metrique::instrument::Instrumented;
use std::time::{Duration, Instant};

// Background queue processor - one metric per processed item
#[metrics(aggregate)]
struct QueueItem {
    #[metrics(key)]
    item_type: &'static str,

    #[metrics(key)]
    priority: u8,

    #[metrics(aggregate = Counter)]
    items_processed: u64,

    #[metrics(aggregate = Histogram)]
    processing_time: Duration,

    #[metrics(aggregate = Counter)]
    processing_errors: u64,
}

async fn setup_queue_processor() {
    // Wrap your normal sink with aggregation
    let base_sink = ServiceMetrics::sink();
    let aggregating_sink = AggregatingEntrySink::with_config(
        base_sink,
        AggregateConfig {
            max_entries: 1000,
            raw_events_per_second: 10.0,  // Sample 10 raw events per second
        }
    );

    // Process queue items as they arrive
    while let Ok(item) = queue.recv().await {
        // Create metrics with append_on_drop for automatic emission
        let mut queue_metrics = QueueItem {
            item_type: item.type_name(),
            priority: item.priority,
            items_processed: 1,
            processing_time: Duration::ZERO,
            processing_errors: 0,
        }.append_on_drop(&aggregating_sink);

        // Process item and capture timing
        let start = Instant::now();
        let result = process_item(item).await;
        queue_metrics.processing_time = start.elapsed();

        if result.is_err() {
            queue_metrics.processing_errors = 1;
        }

        // Metrics automatically aggregated and emitted when dropped
    }

    // Periodically flushes aggregated results + sampled raw events
}
```

**Output**: Multiple aggregated entries like `ItemType: "email", Priority: 1, ItemsProcessed: 1247, ProcessingTime: [histogram], ProcessingErrors: 23`

## Aggregation Strategies

Different field types use different merge strategies:

```rust
use metrique::writer::merge::{Counter, Histogram, Gauge, Max, Min};

#[metrics(aggregate)]
struct MetricExample {
    #[metrics(key)]
    operation: &'static str,  // Part of aggregation key

    #[metrics(aggregate = Counter)]
    total_requests: u64,      // Sums: 1 + 1 + 1 = 3

    #[metrics(aggregate = Histogram)]
    latency: Duration,        // Collects: [50ms, 75ms, 100ms], reports compressed distribution

    #[metrics(aggregate = Gauge)]
    active_connections: u64,  // Last value: 42

    #[metrics(aggregate = Max)]
    peak_memory_mb: u64,      // Maximum: 256

    #[metrics(aggregate = Min)]
    min_latency: Duration,    // Minimum: 45ms
}
```

### Strategy Details

- **Counter**: Sums values - use for counts, totals, accumulated metrics
- **Histogram**: Collects all observations - use for latency, size distributions
- **Gauge**: Keeps last value - use for current state (connections, memory usage)
- **Max/Min**: Tracks extremes - use for peak/minimum values

## Unit Preservation

Units are preserved during aggregation:

```rust
#[metrics(aggregate)]
struct NetworkMetrics {
    #[metrics(aggregate = Counter, unit = Megabyte)]
    bytes_transferred: u64,

    #[metrics(aggregate = Histogram, unit = Millisecond)]
    request_latency: u64,
}
```

Type safety prevents mixing incompatible units - you can't aggregate `Duration` with `u64`.

## Performance Benefits

Metrique's aggregation is extremely efficient because:

- **Zero allocations**: No HashMaps - proc macro generates plain struct code
- **Compile-time structure**: All aggregation logic is generated, not dynamic
- **Type safety**: Prevents runtime errors through compile-time checks

This can be 50x more efficient than HashMap-based approaches.

## Configuration

### Strategy Configuration

For strategies that need runtime configuration (like histogram bucket limits), use the `conf` parameter:

```rust
use metrique::unit_of_work::metrics;
use metrique::writer::aggregate::{Counter, Histogram, HistogramConfig};

// Define your configuration struct
#[derive(Clone)]
struct MyMetricsConfig {
    histogram: HistogramConfig,
}

// Use conf parameter to pass configuration
#[metrics(aggregate(conf = MyMetricsConfig))]
struct RequestMetrics {
    #[metrics(key)]
    operation: &'static str,
    
    #[metrics(aggregate = Counter)]
    request_count: u64,
    
    #[metrics(aggregate = Histogram)]  // Uses config.histogram
    latency: Duration,
}

// Create metrics with configuration
let config = MyMetricsConfig {
    histogram: HistogramConfig { max_buckets: 500 },
};

let metrics = RequestMetrics::new_with_config(
    "get_user", 
    1, 
    Duration::from_millis(50),
    &config
);
```

This enables runtime configuration in libraries like Tokio:
1. **Application startup**: Creates configuration object
2. **Deep in library**: Passes `&config` when creating metrics
3. **No globals needed**: Clean dependency injection

### Embedded Aggregation

Two patterns for adding entries to `Aggregated<T>` fields:

**Direct addition**:
```rust
let mut metrics = TaskMetrics {
    subtasks: Aggregated::new()
};
metrics.subtasks.add(subtask_result);  // Manual aggregation
```

**Append-on-drop pattern** (recommended):
```rust
let mut metrics = TaskMetrics {
    subtasks: Aggregated::new()
};
let mut subtask = metrics.subtasks.append_on_drop();
subtask.field1 = value1;
subtask.field2 = value2;
// Automatically aggregated when subtask drops
```

The `append_on_drop()` method returns a wrapper that implements `DerefMut<Target = T>`, allowing you to mutate fields directly. When the wrapper drops, it automatically adds itself to the aggregator.

### Sink-Level Aggregation

Configure flush behavior and raw event sampling:

```rust
use metrique::writer::merge::{AggregatingEntrySink, AggregateConfig};

let config = AggregateConfig {
    max_entries: 1000,           // Flush when 1000 unique keys accumulated
    raw_events_per_second: 5.0,  // Sample 5 raw events per second for debugging
};

let sink = AggregatingEntrySink::with_config(base_sink, config);
```

## Sampling Integration

Combine aggregation with sampling for the best of both worlds:

```rust
// Automatic sampling with AggregatingEntrySink
let sink = AggregatingEntrySink::with_config(base_sink, AggregateConfig {
    max_entries: 1000,
    raw_events_per_second: 10.0,  // Automatically samples raw events
});
```

This gives you:
- **Precise aggregated metrics**: Exact counts and distributions
- **Raw event samples**: Individual events for tracing and debugging

## Proc Macro Modes

Metrique provides three modes for different aggregation needs:

```rust
// Standard metrics - can be emitted, not aggregated
#[metrics]
struct SimpleMetrics {
    operation: &'static str,
    latency: Duration,
}

// Aggregatable metrics - can be emitted AND aggregated (most common)
#[metrics(aggregate)]
struct RequestMetrics {
    #[metrics(key)]
    operation: &'static str,
    
    #[metrics(aggregate = Counter)]
    request_count: u64,
    
    #[metrics(aggregate = Histogram)]
    latency: Duration,
}

// Aggregation-only - cannot be emitted directly, only aggregated
#[metrics(aggregate_only)]
struct RequestAggregator {
    #[metrics(key)]
    operation: &'static str,
    
    #[metrics(aggregate = Counter)]
    total_requests: u64,
}
```

**Use `#[metrics(aggregate)]` for most cases** - it provides both emission and aggregation capabilities.

## Examples

### TLS Handshake Metrics
```rust
use metrique::writer::merge::{Counter, Histogram};

#[metrics(aggregate)]
struct TlsHandshake {
    #[metrics(key)]
    cipher_suite: &'static str,

    #[metrics(key)]
    tls_version: &'static str,

    #[metrics(aggregate = Counter)]
    handshakes_completed: u64,

    #[metrics(aggregate = Histogram)]
    handshake_duration: Duration,

    #[metrics(aggregate = Counter)]
    handshake_failures: u64,
}
```

### Storage Operation Metrics
```rust
use metrique::writer::merge::{Counter, Histogram};

#[metrics(aggregate)]
struct StorageOp {
> use `#[metrics(value(string))] (read docs if needed) instead of `&'static str` here.
    #[metrics(key)]
    operation_type: &'static str,  // "read", "write", "delete"

    #[metrics(key)]
    storage_tier: &'static str,    // "hot", "warm", "cold"

    #[metrics(aggregate = Counter)]
    operations_count: u64,

    #[metrics(aggregate = Histogram)]
    operation_latency: Duration,

    #[metrics(aggregate = Counter, unit = Byte)]
    bytes_processed: u64,
}
```

## When NOT to Use Aggregation

- **Request/response services**: Use sampling instead
- **Low-frequency events**: Aggregation overhead isn't worth it
- **Need individual event details**: Aggregation loses individual event context
- **Simple counting**: Basic counters don't need aggregation complexity

## Next Steps

- See [Aggregation Internals](aggregated-internals.md) for implementation details
- Learn about [Custom Aggregation Strategies](custom-strategies.md) for ecosystem integration
- Read [Sampling Guide](sampling.md) for the recommended approach for most applications
