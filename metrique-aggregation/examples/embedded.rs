//! Example: Embedded Aggregation Pattern
//!
//! This example demonstrates using `Aggregate<T>` to aggregate multiple sub-operations
//! within a single unit of work. A distributed query fans out to multiple backend shards,
//! and we aggregate all the backend call metrics into a single entry.

use metrique::test_util::test_entry_sink;
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::traits::Aggregate;
use metrique_aggregation::value::Sum;
use metrique_aggregation::{aggregate, histogram::SortAndMerge};
use metrique_writer::unit::Millisecond;
use std::time::Duration;

#[aggregate]
#[metrics]
struct BackendCall {
    #[aggregate(strategy = Sum)]
    requests_made: u64,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = Sum)]
    errors: u64,
}

#[metrics(rename_all = "PascalCase")]
struct DistributedQuery {
    query_id: String,
    #[metrics(flatten)]
    backend_calls: Aggregate<BackendCall>,
}

// Simulated backend call
async fn call_backend(shard: &str, _query: &str) -> Result<String, String> {
    // Simulate varying latencies
    let delay = match shard {
        "shard1" => 45,
        "shard2" => 67,
        "shard3" => 52,
        "shard4" => 71,
        "shard5" => 58,
        _ => 50,
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;

    // Simulate occasional errors
    if shard == "shard3" {
        Err("Connection timeout".to_string())
    } else {
        Ok(format!("Results from {}", shard))
    }
}

async fn execute_distributed_query(query: &str) {
    let sink = test_entry_sink();

    let mut metrics = DistributedQuery {
        query_id: uuid::Uuid::new_v4().to_string(),
        backend_calls: Aggregate::default(),
    };

    println!("Executing query: {}", query);
    println!("Query ID: {}", metrics.query_id);

    // Fan out to 5 backend shards
    for shard in &["shard1", "shard2", "shard3", "shard4", "shard5"] {
        let start = std::time::Instant::now();
        let result = call_backend(shard, query).await;
        let latency = start.elapsed();

        println!(
            "  {} - {}ms - {}",
            shard,
            latency.as_millis(),
            if result.is_ok() { "OK" } else { "ERROR" }
        );

        // Insert each backend call into the aggregator
        metrics.backend_calls.insert(BackendCall {
            requests_made: 1,
            latency,
            errors: if result.is_err() { 1 } else { 0 },
        });
    }

    // Emit the aggregated metrics
    metrics.append_on_drop(sink.sink);

    // Inspect the emitted entry
    let entries = sink.inspector.entries();
    assert_eq!(entries.len(), 1);

    let entry = &entries[0];
    println!("\nEmitted metric entry:");
    println!("  QueryId: {}", entry.values["QueryId"]);
    println!("  RequestsMade: {}", entry.metrics["RequestsMade"].as_u64());
    println!("  Errors: {}", entry.metrics["Errors"].as_u64());
    println!(
        "  Latency distribution: {} observations",
        entry.metrics["Latency"].distribution.len()
    );

    // Verify the aggregation
    assert_eq!(entry.metrics["RequestsMade"].as_u64(), 5);
    assert_eq!(entry.metrics["Errors"].as_u64(), 1);
    assert_eq!(entry.metrics["Latency"].distribution.len(), 5);
}

#[tokio::main]
async fn main() {
    execute_distributed_query("SELECT * FROM users WHERE active = true").await;
}
