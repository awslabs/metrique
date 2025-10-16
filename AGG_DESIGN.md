# Aggregation Proc Macro Implementation

## Goal
Implement proc macro to generate `AggregatableEntry` and `AggregatedEntry` implementations from annotated structs.

## Target API
```rust
#[metrics(aggregate)]
struct RequestMetrics {
    #[metrics(key)]
    operation: &'static str,
    
    #[metrics(key)]
    status_code: u16,
    
    #[metrics(aggregate = Counter)]
    request_count: u64,
    
    #[metrics(aggregate = Histogram)]
    latency_ms: u64,
}
```

## What the Macro Must Generate

### 1. Key Type (tuple of key fields)
```rust
type Key = (&'static str, u16);
```

### 2. Aggregated Struct
```rust
struct AggregatedRequestMetrics {
    key: (&'static str, u16),
    request_count: <Counter as AggregateValue<u64>>::Aggregated,  // u64
    latency_ms: <Histogram as AggregateValue<u64>>::Aggregated,   // VecHistogram
    entry_count: usize,
}
```

### 3. AggregatableEntry Implementation
```rust
impl AggregatableEntry for RequestMetrics {
    type Key = (&'static str, u16);
    type Aggregated = AggregatedRequestMetrics;
    
    fn new_aggregated(key: Self::Key) -> Self::Aggregated {
        AggregatedRequestMetrics {
            key,
            request_count: Counter::init(),
            latency_ms: Histogram::init(),
            entry_count: 0,
        }
    }
    
    fn key(&self) -> Self::Key {
        (self.operation, self.status_code)
    }
}
```

### 4. AggregatedEntry Implementation
```rust
impl AggregatedEntry for AggregatedRequestMetrics {
    type Key = (&'static str, u16);
    type Source = RequestMetrics;
    
    fn aggregate_into(&mut self, entry: &Self::Source) {
        Counter::aggregate(&mut self.request_count, &entry.request_count);
        Histogram::aggregate(&mut self.latency_ms, &entry.latency_ms);
        self.entry_count += 1;
    }
    
    fn count(&self) -> usize {
        self.entry_count
    }
}
```

### 5. Entry Implementation for Aggregated Struct
```rust
impl Entry for AggregatedRequestMetrics {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        // Write key fields
        writer.value("Operation", &self.key.0);
        writer.value("StatusCode", &self.key.1);
        
        // Write aggregated fields
        writer.value("RequestCount", &self.request_count);
        writer.value("LatencyMs", &self.latency_ms);
        writer.value("AggregatedEntryCount", &(self.entry_count as u64));
    }
    
    fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
        [
            ("Operation".into(), self.key.0.into()),
            ("StatusCode".into(), self.key.1.to_string().into()),
        ]
        .into_iter()
    }
}
```

## Implementation Notes

- Parse `#[metrics(aggregate)]` on struct
- Collect fields with `#[metrics(key)]` → build Key tuple type
- Collect fields with `#[metrics(aggregate = Strategy)]` → use Strategy::Aggregated type
- Fields without annotations are ignored (not aggregated)
- Generate struct name: `Aggregated{OriginalName}`
- Keyless aggregation: use `type Key = ()` if no key fields

## Existing Infrastructure

All traits and strategies already exist in `metrique-writer-core/src/merge.rs`:
- `AggregatableEntry`, `AggregatedEntry`, `AggregateValue` traits
- `Counter`, `Gauge`, `Histogram`, `Max`, `Min` strategies
- Working manual examples in `metrique/examples/aggregation-manual.rs`

## Next Steps

1. Add attribute parsing to existing `#[metrics]` macro
2. Generate the 5 implementations above
3. Test with existing examples
