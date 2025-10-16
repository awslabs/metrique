# In-Memory Aggregation Design

## Overview

This document describes the design for in-memory aggregation of metrics in metrique, allowing multiple metric entries to be merged before emission to reduce backend load.

## Core Traits

### `MergeableEntry`

Marks an entry type as capable of being merged:

```rust
pub trait MergeableEntry: Entry {
    type Key: Eq + std::hash::Hash + Clone;
    type Merged: MergedEntry<Source = Self, Key = Self::Key>;
    
    fn new_merged(key: Self::Key) -> Self::Merged;
    fn key(&self) -> Self::Key;
}
```

**Key Type**: Identifies which entries can be merged together. Use `()` for keyless merging where all entries combine regardless of dimensions.

### `MergedEntry`

Accumulates multiple entries of the same type:

```rust
pub trait MergedEntry: Entry {
    type Key: Eq + std::hash::Hash + Clone;
    type Source: MergeableEntry<Merged = Self, Key = Self::Key>;
    
    fn merge_into(&mut self, entry: &Self::Source);
    fn count(&self) -> usize;
}
```

**Note**: The method is named `merge_into` to avoid collision with `Entry::merge()` which combines two different entry types.

## Key Design

The `Key` associated type determines merge behavior:

### Keyed Merging
Entries with the same key merge together:
```rust
struct RequestKey {
    operation: &'static str,
    status_code: u16,
}

impl MergeableEntry for RequestMetrics {
    type Key = RequestKey;
    // ...
}
```

### Keyless Merging
All entries merge together:
```rust
impl MergeableEntry for TotalRequests {
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

## MergingEntrySink

Automatically merges entries with the same key:

```rust
let merging_sink = MergingEntrySink::new(downstream_sink);
merging_sink.append(entry); // Automatically merged by key
```

Configuration options:
- `max_entries`: Flush when this many unique keys accumulated
- `sample_rate`: Emit some unmerged entries for debugging (future)

## Future Work

1. **Proc macro support**: Auto-generate `MergeableEntry` implementations with field-level merge strategy attributes
2. **Sampling**: Emit some percentage of raw entries alongside merged ones
3. **Time-based flushing**: Flush merged entries after a time window
4. **Histogram support**: Proper histogram merging for latency distributions

## Examples

- `metrique/examples/merging-manual.rs` - Keyed merging by operation and status code
- `metrique/examples/merging-keyless.rs` - Keyless merging of all entries

