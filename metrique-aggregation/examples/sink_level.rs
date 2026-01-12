//! Example: Sink-Level Aggregation Pattern
//!
//! This example demonstrates using `WorkerSink` with `KeyedAggregator` for high-rate
//! background processing. A queue processor handles items and aggregates metrics by
//! item type and priority, flushing periodically.

use metrique::ServiceMetrics;
use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::value::ToString;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::value::{MergeOptions, Sum};
use metrique_aggregation::{KeyedAggregator, WorkerSink, aggregate};
use metrique_writer::unit::Millisecond;
use std::time::Duration;
use tokio::sync::mpsc;

#[aggregate]
#[metrics(emf::dimension_sets = [["item_type", "priority"]])]
struct QueueItem {
    #[aggregate(key)]
    item_type: String,

    #[aggregate(key)]
    #[metrics(format = ToString)]
    priority: u8,

    #[aggregate(strategy = Sum)]
    items_processed: u64,

    // NOTE: in the future, I think we can have the proc macro auto-add this for Option<T>
    #[aggregate(strategy = MergeOptions<Histogram<Duration, SortAndMerge>>)]
    #[metrics(unit = Millisecond)]
    processing_time: Option<Duration>,

    #[aggregate(strategy = Sum)]
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
    let keyed_aggregator = KeyedAggregator::<QueueItem>::new(ServiceMetrics::sink());
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
            processing_time: Some(processing_time),
            processing_errors: if result.is_err() { 1 } else { 0 },
        }
        .close_and_merge(worker_sink.clone());
    }

    // Flush remaining aggregated metrics
    println!("\nFlushing final metrics...");
    println!("Emitting EMF metrics to stdout:");
    worker_sink.flush().await;
}

#[tokio::main]
async fn main() {
    // Initialize tracing to see validation errors
    tracing_subscriber::fmt::init();

    // Attach global EMF sink
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::builder("QueueProcessorMetrics".to_string(), vec![vec![]])
            .build()
            .output_to_makewriter(|| std::io::stdout().lock()),
    );

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
