// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Global state patterns for metrics: counters and swappable dimensions.
//!
//! [`GlobalCounter`] is an example of what you can do if you have want to initialize a global
//! static counter or if you are already passing around an `Arc`-wrapped struct.
//!
//! For other most other non-static usage, doing something like [`RequestCounter`] will be more ergonomic
//! and better for testing interactibility.
//!
//! The `ArcSwap` section demonstrates storing server dimensions that can be
//! atomically swapped at runtime (e.g. after a config reload). Requires the
//! `arc-swap` feature on `metrique`.

use std::ops::Deref;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use metrique::emf::Emf;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
global_entry_sink! { ServiceMetrics }

// ---------------------------------------------------------------------------
// Global counter
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// ArcSwap: swappable server dimensions
// ---------------------------------------------------------------------------

/// Process-lifetime server state, readable from any task.
static SERVER_STATE: LazyLock<ArcSwap<ServerStateDimensions>> =
    LazyLock::new(|| ArcSwap::from_pointee(read_server_state()));

#[metrics(subfield_owned, rename_all = "PascalCase")]
#[derive(Clone)]
struct ServerStateDimensions {
    region: String,
    cell: String,
}

fn read_server_state() -> ServerStateDimensions {
    ServerStateDimensions {
        region: std::env::var("AWS_REGION").unwrap_or_else(|_| "us-west-2".into()),
        cell: std::env::var("CELL").unwrap_or_else(|_| "1".into()),
    }
}

// ---------------------------------------------------------------------------
// Metrics struct combining both patterns
// ---------------------------------------------------------------------------

#[metrics(rename_all = "PascalCase")]
struct MyMetrics {
    #[metrics(flatten)]
    server_state: &'static ArcSwap<ServerStateDimensions>,
    in_flight_requests_at_request_start_from_static: Option<u64>,
    in_flight_requests_at_request_start_from_scoped: Option<u64>,
}

impl MyMetrics {
    fn init() -> MyMetricsGuard {
        MyMetrics {
            server_state: &SERVER_STATE,
            in_flight_requests_at_request_start_from_static: None,
            in_flight_requests_at_request_start_from_scoped: None,
        }
        .append_on_drop(ServiceMetrics::sink())
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

    // Simulate a config reload: swap the server state dimensions.
    SERVER_STATE.store(Arc::new(ServerStateDimensions {
        region: "eu-west-1".into(),
        cell: "2".into(),
    }));

    // This request will pick up the new dimensions.
    handle_request(scoped_request_counter).await;
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
