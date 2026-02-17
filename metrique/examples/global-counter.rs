// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This is an example showing global metrics in the form of outstanding requests.
//!
//! In this example, the outstanding requests counter is incremented on request start and
//! decremented on request end. This means that the metric that is emitted at the end shows the
//! outstanding request count at the time the given request entered the system, including itself
//! in that count.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
global_entry_sink! { ServiceMetrics }

struct CounterGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    outstanding_requests: usize,
}

impl RequestMetrics {
    fn init() -> RequestMetricsGuard {
        Self {
            outstanding_requests: 0,
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let outstanding_requests = Arc::new(AtomicUsize::new(0));

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to_makewriter(std::io::stdout),
    );

    let cloned_counter = Arc::clone(&outstanding_requests);
    let handle = tokio::task::spawn(async {
        handle_request(0, cloned_counter).await;
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    handle_request(1, Arc::clone(&outstanding_requests)).await;

    handle.await.unwrap();

    handle_request(2, outstanding_requests).await;

    // EXAMPLE OUTPUT
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"OutstandingRequests"}]}],"Timestamp":1771362327464},"OutstandingRequests":1}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"OutstandingRequests"}]}],"Timestamp":1771362327965},"OutstandingRequests":2}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"OutstandingRequests"}]}],"Timestamp":1771362328968},"OutstandingRequests":1}
    */
}

async fn handle_request(req_num: usize, outstanding_requests: Arc<AtomicUsize>) {
    tracing::info!("request {req_num} started");
    let mut metrics = RequestMetrics::init();
    let count = outstanding_requests.fetch_add(1, Ordering::Relaxed);

    let _guard = CounterGuard {
        counter: Arc::clone(&outstanding_requests),
    };

    metrics.outstanding_requests = count;

    do_some_work().await;

    tracing::info!("request {req_num} finished");
}

async fn do_some_work() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}
