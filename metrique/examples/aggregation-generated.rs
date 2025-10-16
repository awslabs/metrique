// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing generated implementation of AggregatableEntry using the proc macro.

use metrique::unit_of_work::metrics;
use metrique::writer::merge::{Counter, Histogram, AggregatableEntry, AggregatedEntry};

#[metrics(aggregate)]
struct RequestMetrics {
    #[metrics(key)]
    operation: &'static str,
    
    #[metrics(key)]
    status_code: u16,
    
    #[metrics(aggregate = Counter)]
    request_count: u64,
    
    #[metrics(aggregate = Histogram)]
    latency_ms: u64,
}

struct AggregatedMetrics {
    aggregator: AggregatedRequestMetrics
}

fn main() {
    // Create some sample metrics
    let metrics1 = RequestMetrics {
        operation: "GetItem",
        status_code: 200,
        request_count: 1,
        latency_ms: 50,
    };
    
    let metrics2 = RequestMetrics {
        operation: "GetItem",
        status_code: 200,
        request_count: 1,
        latency_ms: 75,
    };
    
    let metrics3 = RequestMetrics {
        operation: "GetItem",
        status_code: 500,
        request_count: 1,
        latency_ms: 200,
    };
    
    // Test aggregation for 200 status code
    let key = metrics1.key();
    let mut aggregated = RequestMetrics::new_aggregated(key);
    
    aggregated.aggregate_into(&metrics1);
    aggregated.aggregate_into(&metrics2);
    
    println!("Aggregated {} entries for status 200", aggregated.count());
    println!("Total requests: {}", aggregated.request_count);
    println!("Average latency: {}ms", aggregated.latency_ms.avg());
    println!("Min latency: {}ms", aggregated.latency_ms.min().unwrap());
    println!("Max latency: {}ms", aggregated.latency_ms.max().unwrap());
    
    // Test aggregation for 500 status code
    let key_500 = metrics3.key();
    let mut aggregated_500 = RequestMetrics::new_aggregated(key_500);
    aggregated_500.aggregate_into(&metrics3);
    
    println!("\nAggregated {} entries for status 500", aggregated_500.count());
    println!("Total requests: {}", aggregated_500.request_count);
    println!("Average latency: {}ms", aggregated_500.latency_ms.avg());
}
