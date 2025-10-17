// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing AggregatingEntrySink with generated aggregation code.

use std::time::Duration;

use metrique::emf::Emf;
use metrique::unit::Megabyte;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    EntrySink, FormatExt,
    merge::{AggregateConfig, AggregatingEntrySink, Counter, VecHistogram},
    sink::FlushImmediately,
};

#[metrics(aggregate)]
struct RequestMetrics {
    #[metrics(key)]
    operation: &'static str,

    #[metrics(key)]
    status_code: u16,

    #[metrics(aggregate = Counter)]
    request_count: u64,

    #[metrics(aggregate = VecHistogram)]
    latency_ms: Duration,

    #[metrics(aggregate = VecHistogram, unit = Megabyte)]
    request_size: u64,
}

fn main() {
    // Create downstream sink for aggregated entries
    let emf_sink = Emf::builder(
        "AggregationSinkExample".to_string(),
        vec![vec!["Operation".to_string()]],
    )
    .build()
    .output_to_makewriter(|| std::io::stdout().lock());

    let flush_sink = FlushImmediately::new(emf_sink);

    // Create aggregating sink with custom config
    let config = AggregateConfig {
        max_entries: 3, // Flush after 3 entries for demo
        sample_rate: 0.0,
    };
    let aggregating_sink = AggregatingEntrySink::with_config(flush_sink, config);

    println!("=== Sending individual entries to aggregating sink ===");

    // Send multiple entries - they should be aggregated
    aggregating_sink.append(RequestMetrics {
        operation: "GetItem",
        status_code: 200,
        request_count: 1,
        latency_ms: Duration::from_millis(50),
        request_size: 10,
    });

    aggregating_sink.append(RequestMetrics {
        operation: "GetItem",
        status_code: 200,
        request_count: 1,
        latency_ms: Duration::from_millis(100),
        request_size: 100,
    });

    aggregating_sink.append(RequestMetrics {
        operation: "GetItem",
        status_code: 500,
        request_count: 1,
        latency_ms: Duration::from_millis(200),
        request_size: 1000,
    });

    // This should trigger flush (max_entries = 3)
    aggregating_sink.append(RequestMetrics {
        operation: "PutItem",
        status_code: 200,
        request_count: 1,
        latency_ms: Duration::from_millis(30),
        request_size: 10000,
    });

    println!("\n=== Flushing remaining entries ===");

    // Flush any remaining entries
    aggregating_sink.flush_aggregated();

    println!("\n=== Done ===");
}
