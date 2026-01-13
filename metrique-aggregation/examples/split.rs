//! Example: Split Aggregation Pattern
//!
//! This example demonstrates using `SplitSink` to send the same data to multiple
//! destinations. We aggregate metrics for precise counts while also emitting raw
//! events for debugging and tracing.

use metrique::test_util::test_entry_sink;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::{EntrySinkAsAggregateSink, SplitSink};
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use std::time::Duration;
use tokio::sync::mpsc;

#[aggregate(ref)]
#[metrics]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(strategy = metrique_aggregation::value::Sum)]
    request_count: u64,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = metrique_aggregation::value::Sum)]
    errors: u64,
}

// Simulated API call
async fn make_api_call(endpoint: &str) -> Result<(), String> {
    // Simulate varying latencies
    let delay = match endpoint {
        "GetUser" => 15,
        "UpdateUser" => 45,
        "DeleteUser" => 30,
        "ListUsers" => 100,
        _ => 25,
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;

    // Simulate occasional errors
    if endpoint == "DeleteUser" && rand::random::<f32>() < 0.2 {
        Err("Permission denied".to_string())
    } else {
        Ok(())
    }
}

async fn api_service(mut requests: mpsc::Receiver<String>) {
    let aggregated_sink = test_entry_sink();
    let raw_sink = test_entry_sink();

    // Create aggregator for precise metrics
    let aggregator = KeyedAggregator::<ApiCall>::new(aggregated_sink.sink);

    // Create raw sink for individual events
    let raw = EntrySinkAsAggregateSink::new(raw_sink.sink);

    // Combine them with SplitSink
    let split = SplitSink::new(aggregator, raw);
    let sink = WorkerSink::new(split, Duration::from_millis(500));

    println!("API service started. Processing requests...\n");

    let mut total_requests = 0;

    while let Some(endpoint) = requests.recv().await {
        let start = std::time::Instant::now();
        let result = make_api_call(&endpoint).await;
        let latency = start.elapsed();

        total_requests += 1;

        println!(
            "Request #{}: {} - {}ms - {}",
            total_requests,
            endpoint,
            latency.as_millis(),
            if result.is_ok() { "OK" } else { "ERROR" }
        );

        // Send to both aggregated and raw sinks
        ApiCall {
            endpoint: endpoint.clone(),
            request_count: 1,
            latency,
            errors: if result.is_err() { 1 } else { 0 },
        }
        .close_and_merge(sink.clone());
    }

    // Flush both sinks
    println!("\nFlushing metrics...");
    sink.flush().await;

    // Inspect aggregated metrics
    let aggregated_entries = aggregated_sink.inspector.entries();
    println!("\n=== Aggregated Metrics ===");
    println!("Total aggregated entries: {}", aggregated_entries.len());

    for entry in &aggregated_entries {
        println!(
            "  Endpoint: {}, Requests: {}, Errors: {}, Latency observations: {}",
            entry.values["endpoint"],
            entry.metrics["request_count"].as_u64(),
            entry.metrics["errors"].as_u64(),
            entry.metrics["latency"].distribution.len()
        );
    }

    // Inspect raw events
    let raw_entries = raw_sink.inspector.entries();
    println!("\n=== Raw Events ===");
    println!("Total raw events: {}", raw_entries.len());

    for (i, entry) in raw_entries.iter().enumerate() {
        println!(
            "  Event #{}: {} - {}ms - {} errors",
            i + 1,
            entry.values["endpoint"],
            entry.metrics["latency"].as_u64(),
            entry.metrics["errors"].as_u64()
        );
    }

    println!("\n=== Summary ===");
    println!(
        "Aggregated entries provide precise counts and distributions across {} unique endpoints",
        aggregated_entries.len()
    );
    println!(
        "Raw events provide individual request details for {} requests",
        raw_entries.len()
    );
}

#[tokio::main]
async fn main() {
    let (tx, rx) = mpsc::channel(100);

    // Spawn the API service
    let service = tokio::spawn(api_service(rx));

    // Simulate incoming API requests
    let requests = vec![
        "GetUser",
        "GetUser",
        "GetUser",
        "UpdateUser",
        "UpdateUser",
        "DeleteUser",
        "DeleteUser",
        "DeleteUser",
        "ListUsers",
        "GetUser",
        "UpdateUser",
        "DeleteUser",
    ];

    for endpoint in requests {
        tx.send(endpoint.to_string()).await.unwrap();
    }

    // Close the channel to signal completion
    drop(tx);

    // Wait for service to finish
    service.await.unwrap();
}
