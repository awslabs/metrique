// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This builds on top of `./unit-of-work-simple.rs` to show ways to pass around
//! child metric structs without needing to muck about with references and lifetimes.
//!
//! This can be useful if eg sending to a blocking thread or otherwise trying to keep your function
//! signatures simple.
//!
//! If you need to also fan out those child metrics to multiple tasks, see `./unit-of-work-fanout.rs`.

use core::default::Default;
use std::time::{Duration, Instant, SystemTime};

use metrique::emf::Emf;
use metrique::writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
use metrique::{OnParentDrop, Slot, SlotGuard, unit_of_work::metrics};
global_entry_sink! { ServiceMetrics }

#[metrics]
struct RequestMetrics {
    // A metric timestamp that we set explicitly to capture the start of the request.
    // If you don't set it, instead it will be implicitly calculated when the metric
    // eventually is flushed, which will be after request processing, any waiting
    // for child tasks to close, any time in a background queue, etc.
    #[metrics(timestamp)]
    timestamp: SystemTime,

    // A slot can be written back to exactly once, from a background task,
    // with mutable access (or else ignored if parent closes before it)
    #[metrics(flatten)]
    response_data: Slot<ResponseData>,

    // You can also make the parent entry wait on the slot closing in the background using
    // `SlotGuard::delay_flush()`
    #[metrics(flatten)]
    more_response_data: Slot<MoreResponseData>,

    #[metrics(flatten)]
    // a slot can be optional, which is cheaper if possibly never opened
    // than `Slot<Option<T>>` inside a slot.
    // FIXME: ???
    post_response_data: Slot<Option<PostResponseData>>,
}

impl RequestMetrics {
    /// The fundamental api for `#[metrics]` is append on drop: You create a new metric struct that is already tied to the relevant sink.
    /// When all handles have been dropped, it is flushed to the sink. This normally would happen at the end of the request, but could happen later.
    ///
    /// This function generates a new instance of the metric, which flushes to ServiceMetrics
    /// when it goes out of scope
    fn init() -> RequestMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            response_data: Default::default(),
            more_response_data: Default::default(),
            post_response_data: Default::default(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[metrics]
struct Keys {
    cluster_id: String,
    operation: &'static str,
}

#[metrics]
#[derive(Default)]
struct ResponseData {
    response_time: Option<Duration>,
    error: usize,
    no_response: bool,
}

#[metrics]
#[derive(Default)]
struct MoreResponseData {
    another_response_time: Duration,
}

#[metrics(subfield)]
#[derive(Default)]
struct PostResponseData {
    post_response_success: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to(std::io::stdout()),
    );
    handle_request().await;
}

async fn handle_request() {
    let mut metrics = RequestMetrics::init();

    // Sometimes you want to split out a segement of metrics to be handled elsewhere as an owned value.
    // You can use `Slot` which is an abstraction over a one shot channel.
    // The data will be written back to the main record
    // when the child slot guard is dropped. This is nice since it lets you keep mutable access without mucking around with references.
    // Note that if the child slot guard isn't dropped, the metric will be omitted.
    let response_metrics = metrics
        .response_data
        .open(OnParentDrop::Discard)
        .expect("slot has not been opened yet");

    let should_make_post_request_call = call_downstream_service(response_metrics).await;

    if should_make_post_request_call {
        // a slot can be optional, in which case there is minimal overhead unless it is instantiated
        // if written as `Option<Slot<T>>`
        let post_response_metrics = metrics
            .post_response_data
            .open(OnParentDrop::Discard)
            .unwrap();
        call_downstream_service_conditional(post_response_metrics).await;
    }

    let flush_guard = metrics.flush_guard();

    // You can also wait to close the parent entry until a slot closes,
    // even without blocking the foreground.
    let more_response_metrics = metrics
        .more_response_data
        .open(OnParentDrop::Wait(flush_guard))
        .expect("response has not been opened");
    tokio::task::spawn(async move {
        call_downstream_service_in_background(more_response_metrics).await;
    });
}

async fn call_downstream_service(mut metrics: SlotGuard<ResponseData>) -> bool {
    metrics.response_time = Some(Duration::from_millis(10));

    true
}

async fn call_downstream_service_in_background(mut metrics: SlotGuard<MoreResponseData>) {
    let start = Instant::now();

    tokio::time::sleep(Duration::from_millis(50)).await;

    metrics.another_response_time = start.elapsed()
}

async fn call_downstream_service_conditional(mut metrics: SlotGuard<Option<PostResponseData>>) {
    metrics.get_or_insert_default().post_response_success = true;
}
