// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing lock-free LocalAggregatingEntrySink for single-threaded performance.

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    EntrySink, FormatExt,
    merge::{Counter, VecHistogram, LocalAggregatingEntrySink, AggregateConfig},
    sink::FlushImmediately,
};

// Use the same pattern as the working examples
#[metrics(aggregate)]
struct ApiMetrics {
    #[metrics(key)]
    endpoint: &'static str,
    
    #[metrics(key)]
    method: &'static str,
    
    #[metrics(aggregate = Counter)]
    request_count: u64,
    
    #[metrics(aggregate = VecHistogram)]
    response_time_ms: u64,
}

fn main() {
    println!("=== Lock-Free Local Aggregating Sink Demo ===\n");
    
    let emf_sink = Emf::builder("LocalApiMetrics".to_string(), vec![vec!["Endpoint".to_string(), "Method".to_string()]])
        .build()
        .output_to_makewriter(|| std::io::stdout().lock());
    
    // Use the lock-free sink - no Arc<Mutex<_>> needed!
    let config = AggregateConfig {
        max_entries: 3,
        sample_rate: 0.0,
    };
    let sink = LocalAggregatingEntrySink::with_config(
        FlushImmediately::new(emf_sink), 
        config
    );
    
    println!("Sending metrics to lock-free sink...\n");
    
    let requests = vec![
        ApiMetrics { endpoint: "/users", method: "GET", request_count: 1, response_time_ms: 45 },
        ApiMetrics { endpoint: "/users", method: "GET", request_count: 1, response_time_ms: 52 },
        ApiMetrics { endpoint: "/users", method: "GET", request_count: 1, response_time_ms: 38 },
        ApiMetrics { endpoint: "/users", method: "POST", request_count: 1, response_time_ms: 120 },
    ];
    
    for (i, request) in requests.into_iter().enumerate() {
        println!("Request {}: {} {} ({}ms)", 
                 i + 1, request.method, request.endpoint, request.response_time_ms);
        println!("  Pending keys before: {}", sink.pending_keys());
        
        sink.append(request);
        
        println!("  Pending keys after: {}", sink.pending_keys());
        println!();
    }
    
    println!("Flushing remaining entries...\n");
    sink.flush_aggregated();
    
    println!("=== Lock-Free Sink Benefits ===");
    println!("✓ No mutex overhead - uses RefCell instead of Mutex");
    println!("✓ No lock contention - single-threaded design"); 
    println!("✓ Better cache locality - no Arc indirection");
    println!("✓ Simpler error handling - no lock poisoning");
    println!("✓ Lower memory overhead - direct ownership");
    println!("✗ Single-threaded only - not Send/Sync");
    println!("\nPerfect for:");
    println!("- Single-threaded applications");
    println!("- Async contexts without thread sharing");
    println!("- Performance-critical paths");
    println!("- Local aggregation before network emission");
}
