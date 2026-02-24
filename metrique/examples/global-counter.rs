// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A global counter, using in-flight requests as an example.
//!
//! [`GlobalCounter`] is an example of what you can do if you have want to initialize a global
//! static counter or if you are already passing around an `Arc`-wrapped struct.
//!
//! For other most other non-static usage, doing something like [`RequestCounter`] will be more ergonomic
//! and better for testing interactibility.

use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
global_entry_sink! { ServiceMetrics }

/// Global static counter to keep track of in-flight requests
static GLOBAL_REQUEST_COUNTER: GlobalCounter = GlobalCounter::new();

#[derive(Default)]
struct GlobalCounter {
    count: AtomicU64,
}
impl GlobalCounter {
    const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }
    /// Increments the count by 1, returning a guard that
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

#[derive(Default, Clone)]
struct RequestCounter {
    base: Arc<GlobalCounter>,
}
impl RequestCounter {
    fn increment(self) -> (RequestCounterGuard, u64) {
        let count = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        (RequestCounterGuard(self), count)
    }
}
impl Deref for RequestCounter {
    type Target = GlobalCounter;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
impl AsRef<GlobalCounter> for RequestCounter {
    fn as_ref(&self) -> &GlobalCounter {
        &self.base
    }
}

struct RequestCounterGuard(RequestCounter);
impl Drop for RequestCounterGuard {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
#[metrics(rename_all = "PascalCase")]
struct MyMetrics {
    in_flight_requests_at_request_start_from_static: Option<u64>,
    in_flight_requests_at_request_start_from_scoped: Option<u64>,
}

impl MyMetrics {
    fn init() -> MyMetricsGuard {
        MyMetrics::default().append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    // An example of the non-static usage
    let scoped_request_counter = RequestCounter::default();

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to_makewriter(std::io::stdout),
    );

    let handle = tokio::task::spawn(handle_request(scoped_request_counter.clone()));
    tokio::time::sleep(Duration::from_millis(500)).await;

    handle_request(scoped_request_counter.clone()).await;
    handle.await.unwrap();
    handle_request(scoped_request_counter).await;

    // EXAMPLE OUTPUT
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"InFlightRequestsAtRequestStartFromStatic"},{"Name":"InFlightRequestsAtRequestStartFromScoped"}]}],"Timestamp":1771884572621},"InFlightRequestsAtRequestStartFromStatic":1,"InFlightRequestsAtRequestStartFromScoped":1}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"InFlightRequestsAtRequestStartFromStatic"},{"Name":"InFlightRequestsAtRequestStartFromScoped"}]}],"Timestamp":1771884573122},"InFlightRequestsAtRequestStartFromStatic":2,"InFlightRequestsAtRequestStartFromScoped":2}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"InFlightRequestsAtRequestStartFromStatic"},{"Name":"InFlightRequestsAtRequestStartFromScoped"}]}],"Timestamp":1771884575124},"InFlightRequestsAtRequestStartFromStatic":1,"InFlightRequestsAtRequestStartFromScoped":1}
    */
}

async fn handle_request(scoped_request_counter: RequestCounter) {
    let mut metrics = MyMetrics::init();

    let (_guard, static_request_count) = GLOBAL_REQUEST_COUNTER.increment();
    let (_guard, scoped_request_count) = scoped_request_counter.increment();
    metrics.in_flight_requests_at_request_start_from_static = Some(static_request_count);
    metrics.in_flight_requests_at_request_start_from_scoped = Some(scoped_request_count);

    do_some_work().await;
}

async fn do_some_work() {
    tokio::time::sleep(Duration::from_secs(2)).await;
}
