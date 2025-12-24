# Metrique Aggregation Internals

This document explains the trait structure and implementation details of metrique's aggregation system.

## Trait Overview

The aggregation system uses these core traits:

**Field-level aggregation**:
- `AggregateValue<T>` - Defines how individual field values combine (Counter, Histogram, etc.)

**Entry-level aggregation**:
- `AggregatableEntry` - Marks entries as aggregatable, defines key extraction
- `AggregatedEntry` - Accumulates multiple entries, implements aggregation logic

**Integration with metrique core**:
- `Entry` - Aggregated entries implement this for output compatibility
- `CloseValue` - Converts aggregated fields to closed form for emission
- `Value` - Aggregated types implement this for metric output

## CloseValue Integration

Aggregated fields integrate with metrique's `CloseValue` system for proper emission:

```rust
impl<T: AggregatableEntry<Key = ()>> CloseValue for Aggregated<T>
where
    T::Aggregated: Entry,
{
    type Closed = Option<T::Aggregated>;

    fn close(self) -> Self::Closed {
        self.aggregated  // Returns the optional aggregated data
    }
}
```

When a metric with `Aggregated<T>` fields is emitted:
1. `CloseValue::close()` is called on the `Aggregated<T>` field
2. This produces `Option<T::Aggregated>`
3. `Option<T>` already implements `Entry` and flattens the aggregated data into the parent metric (if `Some`) or writes nothing (if `None`)

This ensures aggregated fields participate in metrique's standard emission lifecycle using the existing `Option<T>` infrastructure.

## Core Trait Architecture

The aggregation system is built around three main traits that work together to enable type-safe, efficient aggregation:

### `AggregateValue<T>` - Field-Level Aggregation

Defines how individual field values are combined:

```rust
pub trait AggregateValue<T> {
    /// The accumulated type (often same as T, but can differ for histograms)
    type Aggregated;

    /// Initialize a new accumulator
    fn init() -> Self::Aggregated;

    /// Aggregate a value into the accumulator
    fn aggregate(accum: &mut Self::Aggregated, value: &T);
}
```

This trait operates at the field level, not the entry level. Each aggregation strategy (Counter, Histogram, etc.) implements this trait for the types it can aggregate.

**Relationship to metrique core traits**:
- `Self::Aggregated` types implement `Value` for metric output (e.g., `u64` implements `Value`, `HistogramValue<T>` implements `Value`)
- When used in regular metric structs, aggregated fields participate in the `CloseValue` system like any other field
- The aggregated data gets emitted through the standard `Entry::write()` method using the `Value` implementations

**Examples**:
```rust
// Counter sums values
impl<T: Default + AddAssign + Copy> AggregateValue<T> for Counter {
    type Aggregated = T;
    fn init() -> T { T::default() }
    fn aggregate(accum: &mut T, value: &T) { *accum += *value; }
}

// Histogram collects observations
impl<T: MetricValue> AggregateValue<T> for VecHistogram {
    type Aggregated = HistogramValue<T>;
    fn init() -> HistogramValue<T> { HistogramValue::new() }
    fn aggregate(accum: &mut HistogramValue<T>, value: &T) {
        // Uses ValueWriter protocol to collect observations
        let collector = ObservationCollector::new(&mut accum.observations);
        value.write(collector);
    }
}
```

### `AggregatableEntry` - Entry-Level Aggregation

Marks an entry type as capable of being aggregated:

```rust
pub trait AggregatableEntry: Entry {
    /// Key type that identifies which entries can be aggregated together
    type Key: Eq + std::hash::Hash + Clone;

    /// The type that accumulates aggregated entries
    type Aggregated: AggregatedEntry<Source = Self, Key = Self::Key>;

    /// Create a new aggregator for this entry type with the given key
    fn new_aggregated(key: Self::Key) -> Self::Aggregated;

    /// Extract the key from this entry
    fn key(&self) -> Self::Key;
}
```

**Key design decisions**:
- **Key type determines merge behavior**: Entries with the same key get aggregated together
- **Keyless aggregation**: Use `type Key = ()` when all entries should merge regardless of field values
- **Type safety**: The associated `Aggregated` type ensures compile-time correctness

### `AggregatedEntry` - Accumulator Implementation

Accumulates multiple entries and produces aggregated results:

```rust
pub trait AggregatedEntry: Entry {
    /// The key type for this aggregated entry
    type Key: Eq + std::hash::Hash + Clone;

    /// The source entry type being aggregated
    type Source: AggregatableEntry<Aggregated = Self, Key = Self::Key>;

    /// Aggregate another entry into this accumulator
    fn aggregate_into(&mut self, entry: &Self::Source);
}
```

## Proc Macro Design

**Three-mode approach** for maximum flexibility while optimizing the common case:

```rust
#[metrics]                    // Entry + CloseEntry only
#[metrics(aggregate)]         // Entry + CloseEntry + AggregatableEntry + AggregatedEntry  
#[metrics(aggregate_only)]    // AggregatableEntry + AggregatedEntry only
```

**Design rationale**:
- **`#[metrics]`**: Standard metrique behavior - struct can be emitted as metrics
- **`#[metrics(aggregate)]`**: Common case - struct can be both emitted AND aggregated
- **`#[metrics(aggregate_only)]`**: Power user - struct is only for aggregation, not direct emission

**Benefits**:
- **Ergonomic common case**: Most users want both emission and aggregation capabilities
- **Clear intent**: Each mode has obvious semantics
- **Migration friendly**: Easy to add `aggregate` to existing `#[metrics]` structs
- **Composable**: Follows Rust's attribute composition patterns

**Usage examples**:

```rust
// Standard metrics - can be emitted, not aggregated
#[metrics]
struct RequestMetrics {
    operation: &'static str,
    latency: Duration,
}

// Aggregatable metrics - can be emitted AND aggregated
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
    
    #[metrics(aggregate = Histogram)]
    latency_distribution: Duration,
}
```

The `#[metrics(aggregate)]` proc macro generates all the required implementations automatically:

### Input
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
    latency: Duration,
}
```

### Generated Code

The macro generates the aggregated struct and trait implementations, but does not generate a separate key type alias.

**1. Aggregated Struct**
```rust
struct AggregatedRequestMetrics {
    key: (&'static str, u16),
    request_count: <Counter as AggregateValue<u64>>::Aggregated,  // = u64
    latency: <Histogram as AggregateValue<Duration>>::Aggregated, // = HistogramValue<Duration>
    entry_count: usize,
}
```

**2. AggregatableEntry Implementation**
```rust
impl AggregatableEntry for RequestMetrics {
    type Key = (&'static str, u16);
    type Aggregated = AggregatedRequestMetrics;

    fn new_aggregated(key: Self::Key) -> Self::Aggregated {
        AggregatedRequestMetrics {
            key,
            request_count: Counter::init(),
            latency: Histogram::init(),
            entry_count: 0,
        }
    }

    fn key(&self) -> Self::Key {
        (self.operation, self.status_code)
    }
}
```

**3. AggregatedEntry Implementation**
```rust
impl AggregatedEntry for AggregatedRequestMetrics {
    type Key = (&'static str, u16);
    type Source = RequestMetrics;

    fn aggregate_into(&mut self, entry: &Self::Source) {
        Counter::aggregate(&mut self.request_count, &entry.request_count);
        Histogram::aggregate(&mut self.latency, &entry.latency);
        self.entry_count += 1;
    }
}
```

**4. Entry Implementation**
```rust
impl Entry for AggregatedRequestMetrics {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        // Write key fields as dimensions
        writer.value("Operation", &self.key.0);
        writer.value("StatusCode", &self.key.1);

        // Write aggregated fields as metrics
        writer.value("RequestCount", &self.request_count);
        writer.value("Latency", &self.latency);
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

## Aggregation Strategies

Each strategy implements `AggregateValue<T>` for specific aggregation patterns:

### Counter Strategy
```rust
pub struct Counter;

impl<T: Default + AddAssign + Copy> AggregateValue<T> for Counter {
    type Aggregated = T;
    fn init() -> T { T::default() }
    fn aggregate(accum: &mut T, value: &T) { *accum += *value; }
}
```

**Use cases**: Request counts, error counts, bytes transferred

### Histogram Strategy
```rust
pub struct VecHistogram;

impl<T: MetricValue> AggregateValue<T> for VecHistogram {
    type Aggregated = HistogramValue<T>;

    fn init() -> HistogramValue<T> {
        HistogramValue { observations: Vec::new(), _phantom: PhantomData }
    }

    fn aggregate(accum: &mut HistogramValue<T>, value: &T) {
        let collector = ObservationCollector::new(&mut accum.observations);
        value.write(collector);  // Uses ValueWriter protocol
    }
}
```

Histogram aggregation uses the `ValueWriter` protocol to collect observations, preserving unit information and enabling proper statistical analysis.

**Use cases**: Latency distributions, response size distributions

### Other Strategies
- **Gauge**: Keeps last value (current state metrics)
- **Max/Min**: Tracks extremes (peak values)

## Unit Preservation

Units are preserved during aggregation through the type system:

```rust
#[metrics(aggregate = Histogram, unit = Millisecond)]
latency: u64,
```

**Generated field type**:
```rust
latency: <Histogram as AggregateValue<WithUnit<u64, Millisecond>>>::Aggregated,
```

The `WithUnit<T, U>` wrapper ensures:
- Type safety: Can't aggregate incompatible units
- Unit preservation: Output maintains correct unit information
- Compile-time validation: Mixing `Duration` with `u64` causes compile error

## Sink Integration

### AggregatingEntrySink

Automatically aggregates entries by key before forwarding to downstream sink:

```rust
pub struct AggregatingEntrySink<T, S> {
    inner: S,
    state: Mutex<AggregationState<T>>,
    config: AggregateConfig,
}

struct AggregationState<T> {
    aggregated: HashMap<T::Key, T::Aggregated>,
    // ... flush tracking
}
```

**Key operations**:
1. **Append**: Extract key, find or create aggregator, call `aggregate_into`
2. **Flush**: When limits reached, emit all aggregated entries and clear state
3. **Sampling**: Optionally emit raw entries alongside aggregated ones

### Embedded Aggregation (`Aggregated<T>`)

For keyless aggregation within regular metrics:

```rust
pub struct Aggregated<T: AggregatableEntry<Key = ()>> {
    aggregated: Option<T::Aggregated>,
}

impl<T: AggregatableEntry<Key = ()>> Aggregated<T> {
    pub fn add(&mut self, entry: T) {
        match &mut self.aggregated {
            Some(agg) => agg.aggregate_into(&entry),
            None => {
                let mut agg = T::new_aggregated(());
                agg.aggregate_into(&entry);
                self.aggregated = Some(agg);
            }
        }
    }
}
```

**Design constraint**: Only supports keyless aggregation (`Key = ()`) to ensure all entries merge into a single result that can be flattened into the parent metric.

## Performance Characteristics

### Zero-Allocation Design
- **No HashMaps in hot path**: Proc macro generates plain struct field access
- **Compile-time structure**: All aggregation logic is generated, not dynamic
- **Direct field access**: `Counter::aggregate` is just `*accum += *value`

### Memory Efficiency
- **Struct-based storage**: More cache-friendly than HashMap-based approaches
- **Type-specific optimizations**: Each strategy can optimize for its use case
- **Minimal overhead**: Only stores what's needed for each aggregation type

### Comparison to HashMap-Based Approaches
Traditional metrics libraries use `HashMap<String, Value>` storage:
```rust
// Traditional approach
let mut metrics = HashMap::new();
metrics.insert("request_count".to_string(), Value::Counter(1));
metrics.insert("latency_ms".to_string(), Value::Histogram(vec![50]));
```

Metrique's approach:
```rust
// Metrique approach
let mut aggregated = AggregatedRequestMetrics {
    request_count: 1,
    latency: HistogramValue::from_observation(Duration::from_millis(50)),
    // ...
};
```

**Performance benefits**:
- No string allocations or lookups
- No enum dispatch overhead
- Direct memory access patterns
- Compile-time optimization opportunities

## Custom Aggregation Strategies

To implement a custom strategy:

```rust
pub struct CustomStrategy;

impl<T> AggregateValue<T> for CustomStrategy
where
    T: YourTraitBounds
{
    type Aggregated = YourAggregatedType<T>;

    fn init() -> Self::Aggregated {
        // Initialize empty accumulator
    }

    fn aggregate(accum: &mut Self::Aggregated, value: &T) {
        // Your aggregation logic
    }
}
```

**Integration points**:
- Works with existing proc macro: `#[metrics(aggregate = CustomStrategy)]`
- Integrates with unit system: Can work with `WithUnit<T, U>`
- Compatible with sinks: Automatic integration with `AggregatingEntrySink`

**Use cases**:
- OpenTelemetry integration
- Custom statistical measures
- Domain-specific aggregation patterns

## Error Handling

### Compile-Time Safety
- **Type mismatches**: Prevented by trait bounds
- **Unit incompatibility**: Compile error when mixing incompatible units
- **Missing implementations**: Clear error messages when strategy doesn't support a type

### Runtime Behavior
- **Empty aggregation**: `new_aggregated()` creates valid empty state
- **Single entry**: Works normally, `count() == 1`
- **Strategy failures**: Each strategy defines its own error handling (e.g., Gauge panics if no values provided)

## Integration with Core Metrique

The aggregation system builds seamlessly on metrique's existing infrastructure:

- **Entry trait**: Aggregated entries implement `Entry` for output compatibility
- **ValueWriter protocol**: Histogram aggregation uses existing value writing system
- **Unit system**: Full integration with metrique's unit preservation
- **Sink infrastructure**: Works with all existing sink types
- **EMF output**: Transparent compatibility with CloudWatch EMF format

This design ensures aggregation is a pure optimization - it doesn't change the fundamental metrique patterns or output formats.
