// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This shows global metrics using total requests as an example

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
global_entry_sink! { ServiceMetrics }

/// Global to keep track of total requests for the service's uptime
static TOTAL_REQUESTS: LazyLock<Arc<AtomicUsize>> = LazyLock::new(|| Arc::new(AtomicUsize::new(0)));

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    total_requests: usize,
}

impl RequestMetrics {
    fn init() -> RequestMetricsGuard {
        Self { total_requests: 0 }.append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to_makewriter(std::io::stdout),
    );

    handle_request().await;
    handle_request().await;

    // EXAMPLE OUTPUT
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"TotalRequests"}]}],"Timestamp":1771387941273},"TotalRequests":1}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"TotalRequests"}]}],"Timestamp":1771387941273},"TotalRequests":2}
    */
}

async fn handle_request() {
    let mut metrics = RequestMetrics::init();

    metrics.total_requests = TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed) + 1;
}
