# Metrique Aggregation Design RFC

## Overview

Add in-memory aggregation to Metrique using proc macro annotations. Entries with the same key are automatically combined before emission, reducing metric volume while preserving statistical information.

## API Design

### Basic Usage

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
    latency_ms: Duration,
}
```

### Generated Code

For the above struct, the macro generates:

```rust
// Key type (tuple of key field types)
type Key = (&'static str, u16);

// Aggregated struct
#[derive(Debug)]
struct AggregatedRequestMetrics {
    key: (&'static str, u16),
    request_count: <Counter as AggregateValue<u64>>::Aggregated,  // = u64
    latency_ms: <VecHistogram as AggregateValue<Duration>>::Aggregated,  // = HistogramValue<Duration>
    entry_count: usize,
}

// Trait implementations
impl AggregatableEntry for RequestMetrics {
    type Key = (&'static str, u16);
    type Aggregated = AggregatedRequestMetrics;
    
    fn new_aggregated(key: Self::Key) -> Self::Aggregated {
        AggregatedRequestMetrics {
            key,
            request_count: <Counter as AggregateValue<u64>>::init(),  // = 0u64
            latency_ms: <VecHistogram as AggregateValue<Duration>>::init(),  // = HistogramValue { observations: Vec::new(), _phantom }
            entry_count: 0,
        }
    }
    
    fn key(&self) -> Self::Key {
        (self.operation, self.status_code)
    }
}

impl AggregatedEntry for AggregatedRequestMetrics {
    type Key = (&'static str, u16);
    type Source = RequestMetrics;
    
    fn aggregate_into(&mut self, entry: &Self::Source) {
        <Counter as AggregateValue<u64>>::aggregate(&mut self.request_count, &entry.request_count);
        <VecHistogram as AggregateValue<Duration>>::aggregate(&mut self.latency_ms, &entry.latency_ms);
        self.entry_count += 1;
    }
    
    fn count(&self) -> usize {
        self.entry_count
    }
}
```

## Aggregation Strategies

Each strategy defines how values are combined:

```rust
pub trait AggregateValue<T> {
    type Aggregated;
    fn init() -> Self::Aggregated;
    fn aggregate(accum: &mut Self::Aggregated, value: &T);
}

// Counter: sum values
impl<T: Default + AddAssign + Copy> AggregateValue<T> for Counter {
    type Aggregated = T;
    fn init() -> T { T::default() }
    fn aggregate(accum: &mut T, value: &T) { *accum += *value; }
}

// VecHistogram: collect observations using ValueWriter
impl<T: MetricValue> AggregateValue<T> for VecHistogram {
    type Aggregated = HistogramValue<T>;
    fn init() -> HistogramValue<T> { 
        HistogramValue { observations: Vec::new(), _phantom: PhantomData }
    }
    fn aggregate(accum: &mut HistogramValue<T>, value: &T) {
        let collector = ObservationCollector::new(&mut accum.observations);
        value.write(collector);  // Captures actual observations with units
    }
}
```

## Unit Handling

Units are preserved by wrapping field types during aggregation:

```rust
#[metrics(aggregate = Histogram, unit = Megabyte)]
response_size: u64,
```

Generates:
```rust
// Field type becomes WithUnit<u64, Megabyte>
response_size: <Histogram as AggregateValue<WithUnit<u64, Megabyte>>>::Aggregated,

// During aggregation, values are wrapped:
Histogram::aggregate(&mut self.response_size, &entry.response_size.into());
```

## Usage Patterns

### Manual Aggregation

```rust
let key = metrics1.key();  // ("GetItem", 200)
let mut aggregated = RequestMetrics::new_aggregated(key);
aggregated.aggregate_into(&metrics1);  // request_count: 1, latency: [50ms]
aggregated.aggregate_into(&metrics2);  // request_count: 2, latency: [50ms, 75ms]
```

### Keyless Aggregation

```rust
#[metrics(aggregate)]
struct TotalRequests {
    #[metrics(aggregate = Counter)]
    count: u64,
}

// Generates: type Key = ();
// All entries merge together regardless of field values
```

### Edge Cases

- **Empty aggregation**: `new_aggregated()` creates valid empty state
- **Single entry**: Works normally, `entry_count = 1`
- **Zero key fields**: `Key = ()`, all entries merge together
- **Mixed units**: Compile error - cannot aggregate `Duration` with `WithUnit<u64, Second>`

## Working Example

From `aggregation-sink.rs`:

```rust
#[metrics(aggregate)]
struct RequestMetrics {
    #[metrics(key)] operation: &'static str,
    #[metrics(key)] status_code: u16,
    #[metrics(aggregate = Counter)] request_count: u64,
    #[metrics(aggregate = VecHistogram)] latency_ms: Duration,
    #[metrics(aggregate = VecHistogram, unit = Megabyte)] request_size: u64,
}
```

**Input**: 3 entries with keys `("GetItem", 200)`, `("GetItem", 200)`, `("GetItem", 500)`  
**Output**: 2 aggregated entries

```json
{
  "Operation": "GetItem", "StatusCode": 200,
  "RequestCount": 2,
  "LatencyMs": {"Values": [50, 100], "Counts": [1, 1]},
  "RequestSize": {"Values": [10, 100], "Counts": [1, 1], "Unit": "Megabytes"},
  "AggregatedEntryCount": 2
}
```

## Status

- ✅ Proc macro implementation complete
- ✅ All aggregation strategies working  
- ✅ Unit preservation working
- ✅ Integration with AggregatingEntrySink
- ✅ Examples and tests passing
