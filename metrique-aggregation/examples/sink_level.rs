//! Example: Sink-Level Aggregation Pattern
//!
//! This example demonstrates using `WorkerSink` with `KeyedAggregator` for high-rate
//! background processing. A queue processor handles items and aggregates metrics by
//! item type and priority, flushing periodically.

use metrique::test_util::test_entry_sink;
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::{KeyedAggregator, WorkerSink, aggregate};
use metrique_writer::unit::Millisecond;
use std::time::Duration;
use tokio::sync::mpsc;

#[aggregate]
#[metrics]
struct QueueItem {
    #[aggregate(key)]
    item_type: String,

    #[aggregate(key)]
    priority: u8,

    #[aggregate(strategy = metrique_aggregation::value::Sum)]
    items_processed: u64,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    processing_time: Duration,

    #[aggregate(strategy = metrique_aggregation::value::Sum)]
    processing_errors: u64,
}

// Simulated queue item
#[derive(Clone)]
struct Item {
    item_type: String,
    priority: u8,
    data: String,
}

// Simulated item processing
async fn process_item(item: &Item) -> Result<(), String> {
    // Simulate varying processing times based on priority
    let delay = match item.priority {
        1 => 10, // High priority - fast
        2 => 25, // Medium priority
        3 => 50, // Low priority - slow
        _ => 30,
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;

    // Simulate occasional errors for low priority items
    if item.priority == 3 && item.data.contains("error") {
        Err("Processing failed".to_string())
    } else {
        Ok(())
    }
}

async fn queue_processor(mut queue: mpsc::Receiver<Item>) {
    let sink = test_entry_sink();
    let keyed_aggregator = KeyedAggregator::<QueueItem>::new(sink.sink);
    let worker_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(500));

    println!("Queue processor started. Flush interval: 500ms\n");

    let mut total_processed = 0;

    while let Some(item) = queue.recv().await {
        let start = std::time::Instant::now();
        let result = process_item(&item).await;
        let processing_time = start.elapsed();

        total_processed += 1;

        println!(
            "Processed item #{}: type={}, priority={}, time={}ms, result={}",
            total_processed,
            item.item_type,
            item.priority,
            processing_time.as_millis(),
            if result.is_ok() { "OK" } else { "ERROR" }
        );

        // Send metrics to the aggregating sink
        QueueItem {
            item_type: item.item_type.clone(),
            priority: item.priority,
            items_processed: 1,
            processing_time,
            processing_errors: if result.is_err() { 1 } else { 0 },
        }
        .close_and_merge(worker_sink.clone());
    }

    // Flush remaining aggregated metrics
    println!("\nFlushing final metrics...");
    worker_sink.flush().await;

    // Inspect the aggregated entries
    let entries = sink.inspector.entries();
    println!("\nAggregated metric entries: {}", entries.len());

    for entry in entries {
        println!(
            "  Type: {}, Priority: {}, Processed: {}, Errors: {}, Latency observations: {}",
            entry.values["item_type"],
            entry.metrics["priority"].as_u64(),
            entry.metrics["items_processed"].as_u64(),
            entry.metrics["processing_errors"].as_u64(),
            entry.metrics["processing_time"].distribution.len()
        );
    }
}

#[tokio::main]
async fn main() {
    let (tx, rx) = mpsc::channel(100);

    // Spawn the queue processor
    let processor = tokio::spawn(queue_processor(rx));

    // Simulate incoming queue items
    let items = vec![
        Item {
            item_type: "email".to_string(),
            priority: 1,
            data: "Welcome email".to_string(),
        },
        Item {
            item_type: "email".to_string(),
            priority: 1,
            data: "Password reset".to_string(),
        },
        Item {
            item_type: "email".to_string(),
            priority: 2,
            data: "Newsletter".to_string(),
        },
        Item {
            item_type: "notification".to_string(),
            priority: 1,
            data: "Push notification".to_string(),
        },
        Item {
            item_type: "notification".to_string(),
            priority: 2,
            data: "SMS notification".to_string(),
        },
        Item {
            item_type: "report".to_string(),
            priority: 3,
            data: "Monthly report".to_string(),
        },
        Item {
            item_type: "report".to_string(),
            priority: 3,
            data: "Error report - error".to_string(), // Will fail
        },
        Item {
            item_type: "email".to_string(),
            priority: 1,
            data: "Order confirmation".to_string(),
        },
    ];

    for item in items {
        tx.send(item).await.unwrap();
    }

    // Close the channel to signal completion
    drop(tx);

    // Wait for processor to finish
    processor.await.unwrap();
}
