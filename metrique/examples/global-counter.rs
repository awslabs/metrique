// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This shows global metrics using in-flight requests as an example

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
global_entry_sink! { ServiceMetrics }

/// Global counter to keep track of in-flight requests for the service's uptime
static GLOBAL_REQUEST_COUNTER: LazyLock<GlobalCounter> = LazyLock::new(|| GlobalCounter::default());

#[derive(Default)]
struct GlobalCounter {
    count: Arc<AtomicU64>,
}
impl GlobalCounter {
    /// Increments the global count by 1, returning a guard that
    /// decrements the count on drop, and the new value
    fn increment(&'static self) -> (GlobalCounterGuard, u64) {
        let count = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        (GlobalCounterGuard(&self), count)
    }
}

struct GlobalCounterGuard(&'static GlobalCounter);

impl Drop for GlobalCounterGuard {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
#[metrics(rename_all = "PascalCase")]
struct MyMetrics {
    in_flight_requests_at_request_start: Option<u64>,
}

impl MyMetrics {
    fn init() -> MyMetricsGuard {
        MyMetrics::default().append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to_makewriter(std::io::stdout),
    );

    let handle = tokio::task::spawn(handle_request());
    tokio::time::sleep(Duration::from_millis(500)).await;

    handle_request().await;
    handle.await.unwrap();
    handle_request().await;

    // EXAMPLE OUTPUT
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"InFlightRequestsAtRequestStart"}]}],"Timestamp":1771431467669},"InFlightRequestsAtRequestStart":1}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"InFlightRequestsAtRequestStart"}]}],"Timestamp":1771431468171},"InFlightRequestsAtRequestStart":2}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"InFlightRequestsAtRequestStart"}]}],"Timestamp":1771431470173},"InFlightRequestsAtRequestStart":1}
    */
}

async fn handle_request() {
    let mut metrics = MyMetrics::init();

    let (_guard, request_count) = GLOBAL_REQUEST_COUNTER.increment();
    metrics.in_flight_requests_at_request_start = Some(request_count);

    do_some_work().await;
}

async fn do_some_work() {
    tokio::time::sleep(Duration::from_secs(2)).await;
}
