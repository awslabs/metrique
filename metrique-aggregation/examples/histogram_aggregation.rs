//! Example: Aggregating Histograms within Aggregated Structs
//!
//! This example demonstrates aggregating entries where the source type already
//! contains a `Histogram` field. This is useful when sub-operations themselves
//! collect distributions that you want to merge into a single aggregated entry.
//!
//! For example, each backend shard might return latency distributions from its
//! internal processing, and you want to merge all those distributions together.

use metrique::emf::Emf;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::{FormatExt, sink::FlushImmediatelyBuilder};
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::Aggregate;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::value::Sum;
use std::time::Duration;

/// Metrics from a single backend shard, which itself contains a histogram
/// of internal processing latencies observed during that shard's work.
#[aggregate]
#[metrics]
struct ShardResult {
    #[aggregate(strategy = Sum)]
    rows_scanned: usize,

    /// Each shard reports a distribution of per-row processing latencies.
    /// When aggregated, these histograms are merged together.
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    per_row_latency: Histogram<Duration>,
}

/// Top-level query metrics that aggregate results from all shards.
#[metrics(rename_all = "PascalCase")]
struct QueryMetrics {
    query_id: &'static str,
    #[metrics(flatten)]
    shard_results: Aggregate<ShardResult>,
}

fn main() {
    let emf_sink = Emf::builder("HistogramAggregation".to_string(), vec![vec![]])
        .build()
        .output_to_makewriter(|| std::io::stdout().lock());
    let sink = FlushImmediatelyBuilder::new().build_boxed(emf_sink);

    let mut query = QueryMetrics {
        query_id: "q-12345",
        shard_results: Aggregate::default(),
    };

    // Simulate 3 shards, each returning their own latency distribution
    for shard_idx in 0..3 {
        let mut shard = ShardResult {
            rows_scanned: 100,
            per_row_latency: Histogram::default(),
        };
        // Each shard observed some per-row latencies
        for row in 0..10 {
            let latency = Duration::from_micros(200 + (shard_idx * 50) + (row * 10));
            shard.per_row_latency.add_value(latency);
        }
        query.shard_results.insert(shard);
    }

    // Emit — the per_row_latency histograms from all shards are merged
    drop(query.append_on_drop(sink));

    // Output will contain a single entry with:
    // - RowsScanned: 300 (summed across 3 shards)
    // - PerRowLatency: merged distribution of all 30 observations
}
