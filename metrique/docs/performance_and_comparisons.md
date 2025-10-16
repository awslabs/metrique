# Performance and Library Comparisons

This document provides detailed information about Metrique's performance characteristics and how it compares to other metrics libraries in the Rust ecosystem.

## Performance Overview

Metrique is designed for high-performance metrics collection with minimal runtime overhead. The key architectural decisions that enable this performance are:

1. **Compile-time metric definitions**: Metrics are defined as structs with the `#[metrics]` attribute, eliminating runtime hash lookups and dynamic allocations.
2. **Direct serialization**: Metrics are serialized directly from struct fields to output formats without intermediate representations.
3. **Zero-cost abstractions**: When advanced features aren't used, there's no performance penalty compared to manual serialization.

## Performance Characteristics

### Metric Creation Performance

The cost of creating a metric entry with Metrique is extremely low:

- **Typical metric creation**: ~100ns for production-sized metrics
- **Memory allocations**: Minimal to zero for most metric operations
- **CPU overhead**: Primarily limited to struct field assignments and timing operations

This represents significant performance improvements over HashMap-based approaches, which typically require:
- Hash key computation
- Hash table lookups
- Dynamic memory allocation for string keys
- Potential hash collision handling

### Memory Usage

Metrique's memory usage characteristics:

- **Static memory footprint**: Metric definitions don't consume runtime memory
- **Low allocation pressure**: Most operations avoid heap allocations
- **Predictable memory patterns**: No hidden allocations from metric collection
- **Efficient serialization**: Direct struct-to-output formatting minimizes temporary allocations

## Why use `metrique`?

### Instead of  [`metrics`](https://metrics.rs/)

The [`metrics`](https://crates.io/crates/metrics) crate is the most popular metrics library in the Rust ecosystem.

**Metrique advantages:**
- **Performance**: Eliminates HashMap lookups and string key allocations
- **Type safety**: Compile-time metric definition prevents runtime errors
- **Structured output**: Built-in support for structured metric formats (EMF)
- **Lower overhead**: No global registry or metric handle management

**`metrics` crate advantages:**
- **Ecosystem compatibility**: Works with many existing metric backends
- **Simpler mental model**: Similar to metrics libraries in other languages
- **Mature ecosystem**: Extensive integration with other crates

**Performance comparison:**
```rust
// metrics crate - requires runtime lookups
counter!("requests", "status" => "200", "method" => "GET").increment(1);

// Metrique - compile-time definition, direct serialization
let mut request_metrics = RequestMetrics::init();
request_metrics.status = "200";
request_metrics.method = "GET";
request_metrics.count = 1;
// Automatic emission on drop
```

### vs. `prometheus` Crate

The [`prometheus`](https://crates.io/crates/prometheus) crate provides Prometheus-compatible metrics.

**Metrique advantages:**
- **Lower memory overhead**: No metric family management or label validation overhead
- **Direct export**: Bypasses intermediate aggregation for unit-of-work metrics
- **Better performance**: Eliminates global metric registry operations

**Prometheus crate advantages:**
- **Standard format**: Direct Prometheus compatibility
- **Metric families**: Built-in support for grouped metrics
- **Aggregation**: Client-side metric aggregation and exposition

**Use case differences:**
- **Metrique**: Best for unit-of-work metrics and direct CloudWatch integration
- **Prometheus**: Best for traditional monitoring metrics and Prometheus/Grafana stacks

### vs. Custom Structured Logging

Many applications use structured logging (like `tracing` or `slog`) for metrics.

**Metrique advantages:**
- **Optimized serialization**: Format-specific optimizations (EMF, etc.)
- **Metric semantics**: Built-in understanding of dimensions, units, timestamps
- **Type safety**: Compile-time validation of metric structure
- **Performance**: Optimized specifically for metric emission patterns

**Structured logging advantages:**
- **Flexibility**: Can handle arbitrary data structures
- **Unified tooling**: Same infrastructure for logs and metrics
- **Rich context**: Easy to include detailed contextual information

### vs. Low-Level Serialization Libraries

Some applications build metrics systems on top of `serde_json` or similar serialization libraries.

**Metrique advantages:**
- **Domain-specific optimizations**: Optimized for metric emission patterns
- **Built-in metric concepts**: Dimensions, units, timestamps handled automatically
- **Format compliance**: Ensures output conforms to metric format specifications
- **Higher-level abstractions**: Slots, guards, and other convenience features

**Low-level libraries advantages:**
- **Maximum control**: Complete control over output format
- **Minimal dependencies**: Fewer dependencies in your application
- **Flexibility**: Can adapt to any output format requirements

## When to Choose Metrique

### Ideal Use Cases

Metrique is particularly well-suited for:

1. **High-throughput services**: Where metric collection overhead matters
2. **CloudWatch integration**: Direct EMF output for AWS environments
3. **Unit-of-work metrics**: Tracking individual request/operation metrics
4. **Type-safe metrics**: Applications that benefit from compile-time validation
5. **Latency-sensitive applications**: Where every microsecond of overhead matters

### Consider Alternatives When

You might prefer other solutions when you need:

1. **Prometheus compatibility**: Direct integration with Prometheus/Grafana
2. **Client-side aggregation**: Aggregating metrics before export
3. **Maximum flexibility**: Arbitrary metric structures and formats
4. **Ecosystem integration**: Heavy use of existing `metrics` crate integrations
5. **Simple counter/gauge patterns**: Basic metric types without structured dimensions

## Performance Best Practices

### Optimizing Metric Creation

1. **Reuse metric structs**: Where possible, initialize once and mutate fields
2. **Avoid string allocations**: Use `&'static str` for constant dimension values
3. **Batch operations**: Use slots for metrics spanning multiple async operations
4. **Minimize timer overhead**: Only use timers when timing precision is needed

### Memory Management

1. **Prefer stack allocation**: Most metric operations should avoid heap allocation
2. **Use appropriate sink types**: Choose between immediate flush and background queuing based on performance needs
3. **Configure queue sizes**: Size background queues appropriately for your throughput

### Serialization Performance

1. **Choose appropriate formats**: EMF is optimized for CloudWatch, but has some overhead
2. **Minimize dimension cardinality**: High-cardinality dimensions can impact downstream systems
3. **Use efficient I/O**: Choose appropriate output destinations for your deployment platform

## Measuring Performance

To validate Metrique's performance in your specific use case:

1. **Benchmark metric creation**: Time the creation and emission of your typical metrics
2. **Monitor memory usage**: Check for unexpected allocations or memory growth
3. **Profile hot paths**: Use profiling tools to identify any performance bottlenecks
4. **Load test**: Validate performance under realistic load conditions

Example benchmark:

```rust
use std::time::Instant;
use metrique::unit_of_work::metrics;

#[metrics]
struct BenchmarkMetric {
    operation: &'static str,
    status_code: u16,
    duration_ms: u64,
    #[metrics(timestamp)]
    timestamp: metrique::Timestamp,
}

fn benchmark_metric_creation() {
    let start = Instant::now();

    for _ in 0..10000 {
        let _guard = BenchmarkMetric {
            operation: "test_operation",
            status_code: 200,
            duration_ms: 42,
            timestamp: metrique::Timestamp::now(),
        }.append_on_drop(test_sink());
    }

    let elapsed = start.elapsed();
    println!("Average per metric: {:?}", elapsed / 10000);
}
```

## Format-Specific Performance Considerations

### EMF (Embedded Metric Format)

- **JSON serialization overhead**: EMF requires JSON output, which has some serialization cost
- **Dimension validation**: Validation can be expensive; disable in production if not needed
- **CloudWatch limits**: Be aware of CloudWatch's dimension and metric limits

### Future Formats

As additional format support is added to Metrique, performance characteristics may vary:
- **Binary formats**: May offer better serialization performance
- **Prometheus format**: May require different optimization strategies
- **Custom formats**: Can be optimized for specific use cases

For the most up-to-date performance information and benchmarks, see the `benches/` directory in the Metrique repository.
