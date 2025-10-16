// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Comprehensive example exploring AggregatingEntrySink capabilities.

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    EntrySink, FormatExt,
    merge::{Counter, Histogram, Gauge, Max, Min, AggregatingEntrySink, AggregateConfig},
    sink::FlushImmediately,
};

#[metrics(aggregate)]
struct ServerMetrics {
    #[metrics(key)]
    service: &'static str,
    
    #[metrics(key)]
    region: &'static str,
    
    #[metrics(aggregate = Counter)]
    request_count: u64,
    
    #[metrics(aggregate = Histogram)]
    response_time_ms: u64,
    
    #[metrics(aggregate = Gauge)]
    active_connections: u32,
    
    #[metrics(aggregate = Max)]
    peak_memory_mb: u64,
    
    #[metrics(aggregate = Min)]
    min_cpu_usage: u8,
}

fn main() {
    println!("=== Comprehensive Aggregating Sink Demo ===\n");
    
    // Create sink with small batch size for demo
    let emf_sink = Emf::builder("ServerMetrics".to_string(), vec![vec!["Service".to_string(), "Region".to_string()]])
        .build()
        .output_to_makewriter(|| std::io::stdout().lock());
    
    let config = AggregateConfig {
        max_entries: 5,  // Small batch for demo
        sample_rate: 0.0,
    };
    let sink = AggregatingEntrySink::with_config(FlushImmediately::new(emf_sink), config);
    
    println!("Sending metrics from multiple servers...\n");
    
    // Simulate metrics from multiple servers
    let metrics = vec![
        // API service in us-east-1
        ServerMetrics {
            service: "api",
            region: "us-east-1", 
            request_count: 100,
            response_time_ms: 50,
            active_connections: 25,
            peak_memory_mb: 512,
            min_cpu_usage: 20,
        },
        ServerMetrics {
            service: "api",
            region: "us-east-1",
            request_count: 150,
            response_time_ms: 75,
            active_connections: 30,  // Gauge will keep this value
            peak_memory_mb: 600,     // Max will keep this value
            min_cpu_usage: 15,       // Min will keep this value
        },
        // API service in us-west-2 (different key)
        ServerMetrics {
            service: "api",
            region: "us-west-2",
            request_count: 80,
            response_time_ms: 45,
            active_connections: 20,
            peak_memory_mb: 400,
            min_cpu_usage: 25,
        },
        // Database service in us-east-1 (different key)
        ServerMetrics {
            service: "database",
            region: "us-east-1",
            request_count: 50,
            response_time_ms: 120,
            active_connections: 10,
            peak_memory_mb: 1024,
            min_cpu_usage: 40,
        },
        // More API metrics for us-east-1 (will aggregate with first two)
        ServerMetrics {
            service: "api",
            region: "us-east-1",
            request_count: 75,
            response_time_ms: 60,
            active_connections: 28,  // Gauge will keep this final value
            peak_memory_mb: 550,     // Max keeps 600 (previous max)
            min_cpu_usage: 18,       // Min keeps 15 (previous min)
        },
    ];
    
    // Send all metrics
    for (i, metric) in metrics.into_iter().enumerate() {
        println!("Sending metric {} - Service: {}, Region: {}", 
                 i + 1, metric.service, metric.region);
        sink.append(metric);
    }
    
    println!("\nFlushing remaining aggregated entries...\n");
    sink.flush_aggregated();
    
    println!("\n=== Analysis ===");
    println!("Expected aggregation:");
    println!("1. api/us-east-1: 3 entries aggregated");
    println!("   - RequestCount: 100 + 150 + 75 = 325 (Counter)");
    println!("   - ResponseTimeMs: [50, 75, 60] (Histogram)");
    println!("   - ActiveConnections: 28 (Gauge - last value)");
    println!("   - PeakMemoryMb: 600 (Max of 512, 600, 550)");
    println!("   - MinCpuUsage: 15 (Min of 20, 15, 18)");
    println!("2. api/us-west-2: 1 entry (no aggregation)");
    println!("3. database/us-east-1: 1 entry (no aggregation)");
}
