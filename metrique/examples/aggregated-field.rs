// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing Aggregated<T> field within regular metrics for fan-out patterns.

use std::time::Duration;

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, EntrySink, FormatExt, GlobalEntrySink,
    merge::{Counter, VecHistogram},
    sink::global_entry_sink,
};
use metrique::{Aggregated, CloseValue, RootEntry};

global_entry_sink! { ServiceMetrics }

// Main task metrics with embedded aggregated subtask results
#[metrics]
struct TaskResults {
    task_id: &'static str,

    #[metrics(flatten)]
    subtask_metrics: Aggregated<SubtaskMetrics>,
}

// Keyless aggregation for subtask metrics
#[metrics(aggregate)]
struct SubtaskMetrics {
    // No #[metrics(key)] fields - all entries merge together
    #[metrics(aggregate = Counter)]
    processed_items: u64,

    #[metrics(aggregate = VecHistogram)]
    processing_time: Duration,

    #[metrics(aggregate = Counter)]
    errors_encountered: u64,
}

fn main() {
    tracing_subscriber::fmt::init();
    // Initialize metrics sink
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::builder(
            "AggregatedFieldExample".to_string(),
            vec![vec!["task_id".to_string()]],
        )
        .build()
        .output_to_makewriter(|| std::io::stdout().lock()),
    );

    println!("=== Fan-out Task Example ===\n");

    // Simulate a main task that fans out to multiple subtasks
    let mut task_results = TaskResults {
        task_id: "main_processing_task",
        subtask_metrics: Aggregated::new(),
    }
    .append_on_drop(ServiceMetrics::sink());

    // Simulate subtask 1: parsing
    println!("Subtask 1: Parsing 100 items in 50ms");
    task_results.subtask_metrics.add(SubtaskMetrics {
        processed_items: 100,
        processing_time: Duration::from_millis(50),
        errors_encountered: 2,
    });

    // Simulate subtask 2: validation
    println!("Subtask 2: Validating 150 items in 75ms");
    task_results.subtask_metrics.add(SubtaskMetrics {
        processed_items: 150,
        processing_time: Duration::from_millis(75),
        errors_encountered: 1,
    });

    // Simulate subtask 3: transformation
    println!("Subtask 3: Transforming 200 items in 30ms");
    task_results.subtask_metrics.add(SubtaskMetrics {
        processed_items: 200,
        processing_time: Duration::from_millis(30),
        errors_encountered: 0,
    });

    println!("\n=== Emitting aggregated results ===");

    // Debug: check if we have aggregated data
    if let Some(agg) = task_results.subtask_metrics.get() {
        //println!("Aggregated {} entries", agg.count());
        println!("Total processed items: {}", agg.processed_items);
        println!("Total errors: {}", agg.errors_encountered);
        println!(
            "Processing time observations: {}",
            agg.processing_time.count()
        );
    } else {
        println!("No aggregated data!");
    }

    drop(task_results);

    println!("\n=== Expected Output ===");
    println!("TaskId: main_processing_task");
    println!("ProcessedItems: 450 (100 + 150 + 200)");
    println!("ProcessingTime: [50ms, 75ms, 30ms] histogram");
    println!("ErrorsEncountered: 3 (2 + 1 + 0)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrique::writer::merge::AggregatedEntry;

    #[test]
    fn test_aggregated_field() {
        let mut task_results = TaskResults {
            task_id: "test_task",
            subtask_metrics: Aggregated::new(),
        };

        // Initially empty
        assert!(task_results.subtask_metrics.get().is_none());

        // Add first subtask
        task_results.subtask_metrics.add(SubtaskMetrics {
            processed_items: 100,
            processing_time: Duration::from_millis(50),
            errors_encountered: 2,
        });

        // Should have aggregated entry now
        let agg = task_results.subtask_metrics.get().unwrap();
        assert_eq!(agg.count(), 1);

        // Add second subtask
        task_results.subtask_metrics.add(SubtaskMetrics {
            processed_items: 150,
            processing_time: Duration::from_millis(75),
            errors_encountered: 1,
        });

        // Should have aggregated both
        let agg = task_results.subtask_metrics.get().unwrap();
        assert_eq!(agg.count(), 2);
        assert_eq!(agg.processed_items, 250); // 100 + 150
        assert_eq!(agg.errors_encountered, 3); // 2 + 1
        assert_eq!(agg.processing_time.count(), 2); // Two observations
    }

    #[test]
    fn test_empty_aggregated_field() {
        let task_results = TaskResults {
            task_id: "empty_task",
            subtask_metrics: Aggregated::new(),
        };

        // Should handle empty case gracefully
        assert!(task_results.subtask_metrics.get().is_none());
    }
}
