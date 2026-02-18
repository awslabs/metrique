// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This example demonstrates using MultiFlex to emit metrics for dynamic lists of items.
//!
//! MultiFlex is useful when you have a variable number of similar items to track metrics for,
//! such as multiple database connections, API endpoints, or processing stages.

use metrique::{
    emf::Emf,
    multi_flex::{FlexItem, MultiFlex},
    unit_of_work::metrics,
    writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink},
};
use std::{borrow::Cow, fmt::Write};

global_entry_sink! { ServiceMetrics }

#[metrics(subfield)]
#[derive(Clone, Debug)]
struct RequestMetrics {
    request_id: usize,
    #[metrics(flatten, prefix = "endpoints")]
    endpoints: MultiFlex<EndpointCall>,
    total_calls: usize,
}

#[metrics]
struct MultiRequest {
    #[metrics(flatten)]
    requests: MultiFlex<RequestMetrics>,
}

impl FlexItem for RequestMetrics {
    fn prefix_item(&self, _idx: usize, mut buffer: impl Write) {
        write!(buffer, "{}.", self.request_id).unwrap();
    }
}

#[metrics(subfield)]
#[derive(Debug, Clone)]
struct EndpointCall {
    name: usize,
    response_time_ms: u64,
    status_code: u16,
}

impl FlexItem for EndpointCall {
    fn prefix_item(&self, idx: usize, mut buffer: impl Write) {
        write!(buffer, ".{idx}.").unwrap();
    }
}

fn main() {
    // Initialize metrics sink
    tracing_subscriber::fmt::init();
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::builder(
            "MultiFlexExample".to_string(),
            vec![vec!["request_id".to_string()]],
        )
        .skip_all_validations(true)
        .build()
        .output_to_makewriter(|| std::io::stdout().lock()),
    );

    // Create metrics for a request that calls multiple endpoints
    let mut request_metrics = RequestMetrics {
        request_id: 12345,
        endpoints: MultiFlex::default(),
        total_calls: 0,
    };

    // Add metrics for each endpoint call
    request_metrics.endpoints.push(EndpointCall {
        name: 1,
        response_time_ms: 45,
        status_code: 200,
    });

    request_metrics.endpoints.push(EndpointCall {
        name: 2,
        response_time_ms: 12,
        status_code: 200,
    });

    request_metrics.endpoints.push(EndpointCall {
        name: 3,
        response_time_ms: 89,
        status_code: 503,
    });

    request_metrics.total_calls = 3;

    let mut multi_requset = MultiRequest {
        requests: Default::default(),
    };
    multi_requset.requests.push(request_metrics.clone());
    let mut other_request = request_metrics.clone();
    other_request.request_id = 67890;
    multi_requset.requests.push(other_request);

    // Emit the metrics - this will create metrics like:
    // endpoints.0.response_time_ms: 45
    // endpoints.1.response_time_ms: 12
    // endpoints.2.response_time_ms: 89
    multi_requset.append_on_drop(ServiceMetrics::sink());

    println!("Metrics emitted for request with {} endpoint calls", 3);
}
