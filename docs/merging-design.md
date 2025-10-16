# In-Memory Aggregation Design

## Overview

This document describes the design for in-memory aggregation of metrics in metrique, allowing multiple metric entries to be merged before emission to reduce backend load.

## Core Traits

### `MergeableEntry`

Marks an entry type as capable of being merged:

```rust
pub trait MergeableEntry: Entry {
    type Merged: MergedEntry<Source = Self>;
    fn new_merged() -> Self::Merged;
}
```

### `MergedEntry`

Accumulates multiple entries of the same type:

```rust
pub trait MergedEntry: Entry {
    type Source: MergeableEntry<Merged = Self>;
    fn merge_into(&mut self, entry: &Self::Source);
    fn count(&self) -> usize;
}
```

**Note**: The method is named `merge_into` to avoid collision with `Entry::merge()` which combines two different entry types.

## Merge Strategies

Different field types require different merge strategies:

- **Counters** (request_count, error_count): Sum values
- **Timers/Latencies** (total_latency_ms): Sum for total, divide by count for average
- **Gauges** (active_connections): Last value, min, max, or average
- **Dimensions** (operation, status_code): Must match for entries to merge

## MergingEntrySink

Automatically merges entries with the same sample group:

```rust
let merging_sink = MergingEntrySink::new(downstream_sink);
merging_sink.append(entry); // Automatically merged by sample group
```

Configuration options:
- `max_entries`: Flush when this many unique sample groups accumulated
- `sample_rate`: Emit some unmerged entries for debugging (future)

## Sample Groups

Entries are merged based on their `sample_group()` - typically dimensions like operation and status code. Only entries with identical sample groups are merged together.

## Future Work

1. **Proc macro support**: Auto-generate `MergeableEntry` implementations with field-level merge strategy attributes
2. **Sampling**: Emit some percentage of raw entries alongside merged ones
3. **Time-based flushing**: Flush merged entries after a time window
4. **Histogram support**: Proper histogram merging for latency distributions

## Example

See `metrique/examples/merging-manual.rs` for a complete manual implementation.
