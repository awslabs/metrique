# In-Memory Aggregation Design

## Overview

This document describes the design for in-memory aggregation of metrics in metrique, allowing multiple metric entries to be aggregated before emission to reduce backend load.

## Core Traits

### `AggregatableEntry`

Marks an entry type as capable of being aggregated:

```rust
pub trait AggregatableEntry: Entry {
    type Key: Eq + std::hash::Hash + Clone;
    type Aggregated: AggregatedEntry<Source = Self, Key = Self::Key>;
    
    fn new_aggregated(key: Self::Key) -> Self::Aggregated;
    fn key(&self) -> Self::Key;
}
```

**Key Type**: Identifies which entries can be aggregated together. Use `()` for keyless aggregation where all entries combine regardless of dimensions.

### `AggregatedEntry`

Accumulates multiple entries of the same type:

```rust
pub trait AggregatedEntry: Entry {
    type Key: Eq + std::hash::Hash + Clone;
    type Source: AggregatableEntry<Aggregated = Self, Key = Self::Key>;
    
    fn aggregate_into(&mut self, entry: &Self::Source);
    fn count(&self) -> usize;
}
```

**Note**: The method is named `aggregate_into` to avoid collision with `Entry::merge()` which combines two different entry types.

## Key Design

The `Key` associated type determines merge behavior:

### Keyed Aggregation
Entries with the same key merge together:
```rust
struct RequestKey {
    operation: &'static str,
    status_code: u16,
}

impl AggregatableEntry for RequestMetrics {
    type Key = RequestKey;
    // ...
}
```

### Keyless Aggregation
All entries merge together:
```rust
impl AggregatableEntry for TotalRequests {
    type Key = ();  // No key
    
    fn key(&self) -> Self::Key {
        ()  // Always returns unit
    }
}
```

## Merge Strategies

Different field types require different merge strategies:

- **Counters** (request_count, error_count): Sum values
- **Timers/Latencies** (total_latency_ms): Sum for total, divide by count for average
- **Gauges** (active_connections): Last value, min, max, or average
- **Keys/Dimensions** (operation, status_code): Part of the Key type, set during construction

## AggregationEntrySink

Automatically merges entries with the same key:

```rust
let aggregation_sink = AggregationEntrySink::new(downstream_sink);
aggregation_sink.append(entry); // Automatically aggregated by key
```

Configuration options:
- `max_entries`: Flush when this many unique keys accumulated
- `sample_rate`: Emit some unaggregated entries for debugging (future)

## Future Work

1. **Proc macro support**: Auto-generate `AggregatableEntry` implementations with field-level merge strategy attributes
2. **Sampling**: Emit some percentage of raw entries alongside aggregated ones
3. **Time-based flushing**: Flush aggregated entries after a time window
4. **Histogram support**: Proper histogram aggregation for latency distributions

## Examples

- `metrique/examples/aggregation-manual.rs` - Keyed aggregation by operation and status code
- `metrique/examples/aggregation-keyless.rs` - Keyless aggregation of all entries

