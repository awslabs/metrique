// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Working example of AggregatingEntrySink with multiple aggregation strategies.

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    EntrySink, FormatExt,
    merge::{Counter, VecHistogram, Max, Min, AggregatingEntrySink, AggregateConfig},
    sink::FlushImmediately,
};

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
    
    #[metrics(aggregate = Max)]
    max_response_size: u64,
    
    #[metrics(aggregate = Min)]
    min_response_size: u64,
}

fn main() {
    println!("=== API Metrics Aggregation Demo ===\n");
    
    let emf_sink = Emf::builder("ApiMetrics".to_string(), vec![vec!["Endpoint".to_string(), "Method".to_string()]])
        .build()
        .output_to_makewriter(|| std::io::stdout().lock());
    
    let config = AggregateConfig {
        max_entries: 4,  // Flush after 4 unique keys
        sample_rate: 0.0,
    };
    let sink = AggregatingEntrySink::with_config(FlushImmediately::new(emf_sink), config);
    
    println!("Sending API request metrics...\n");
    
    // Simulate API requests
    let requests = vec![
        // GET /users requests
        ApiMetrics {
            endpoint: "/users",
            method: "GET",
            request_count: 1,
            response_time_ms: 45,
            max_response_size: 1024,
            min_response_size: 1024,
        },
        ApiMetrics {
            endpoint: "/users", 
            method: "GET",
            request_count: 1,
            response_time_ms: 52,
            max_response_size: 2048,  // This will be the max
            min_response_size: 2048,
        },
        ApiMetrics {
            endpoint: "/users",
            method: "GET", 
            request_count: 1,
            response_time_ms: 38,
            max_response_size: 1536,
            min_response_size: 1536,  // This will be between min and max
        },
        // POST /users request (different key)
        ApiMetrics {
            endpoint: "/users",
            method: "POST",
            request_count: 1,
            response_time_ms: 120,
            max_response_size: 512,
            min_response_size: 512,
        },
        // GET /orders request (different key)
        ApiMetrics {
            endpoint: "/orders",
            method: "GET",
            request_count: 1,
            response_time_ms: 75,
            max_response_size: 4096,
            min_response_size: 4096,
        },
    ];
    
    for (i, request) in requests.into_iter().enumerate() {
        println!("Request {}: {} {} ({}ms, {} bytes)", 
                 i + 1, request.method, request.endpoint, 
                 request.response_time_ms, request.max_response_size);
        sink.append(request);
    }
    
    println!("\nFlushing remaining entries...\n");
    sink.flush_aggregated();
    
    println!("=== Expected Results ===");
    println!("GET /users: 3 requests aggregated");
    println!("  - Total requests: 3");
    println!("  - Response times: [45, 52, 38] ms");
    println!("  - Max response size: 2048 bytes");
    println!("  - Min response size: 1024 bytes");
    println!("POST /users: 1 request (no aggregation)");
    println!("GET /orders: 1 request (no aggregation)");
}
