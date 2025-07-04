// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This example builds on top of `./unit-of-work-example.rs`
//! to show options for using this library to handle fan-out/fan-in when you want
//! to write to the same entry from multiple tasks.

use core::default::Default;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use metrique::{CloseValue, Counter, SharedChild, Slot, unit_of_work::metrics};
use metrique_writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
use metrique_writer_format_emf::Emf;
global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    request_id: String,

    // A metric timestamp that we set explicitly to capture the start of the request.
    // If you don't set it, instead it will be implicitly calculated when the metric
    // eventually is flushed, which will be after request processing, any waiting
    // for child tasks to close, any time in a background queue, etc.
    #[metrics(timestamp)]
    timestamp: SystemTime,

    // An example counter that has dimensions added to it
    // We use a Counter so that we can increment it even after making
    // the parent entry immutable and clonable via `RequestMetrics::handle()`.
    counter: Counter,

    // Usually our counters don't need to be clonable since
    // they are stored in a slot or handle, but if you want
    // to clone and send them somewhere directly without eg a Slot, you can wrap them in an Arc.
    // They will just use the state at the time of closing regardless of handles being open.
    max_response_duration_before_foreground_closes_millis: Arc<Counter>,

    // If you want to clone around an entry and write to it with &self,
    // and you know all references will close before the parent (or are ok with contained metrics
    // being omitted), you can use `SharedChild`. SharedChild is backed by a simple Arc so is very cheap.
    #[metrics(flatten)]
    shared_child: SharedChild<ResponseData>,

    // If you want to clone around an entry and write to it with &self, but you aren't sure
    // if the background tasks will finish before the parent, you can use a `Slot`
    // and instead wrap its SlotGuard into Arc<SlotGuard> after using `SlotGuard::delay_flush()`.
    // This is still fairly cheap and gives you a much richer API for making `&mut self`` writes
    // before wrapping in an Arc, managing flushing, etc.
    #[metrics(flatten)]
    slot: Slot<MoreResponseData>,
}

impl RequestMetrics {
    /// The fundamental api for `#[metrics]` is append on drop: You create a new metric struct that is already tied to the relevant sink.
    /// When all handles have been dropped, it is flushed to the sink. This normally would happen at the end of the request, but could happen later.
    ///
    /// This function generates a new instance of the metric, which flushes to ServiceMetrics
    /// when it goes out of scope
    fn init(request_id: &str) -> RequestMetricsGuard {
        let request_id = request_id.to_owned();
        Self {
            request_id,
            timestamp: SystemTime::now(),
            counter: Default::default(),
            max_response_duration_before_foreground_closes_millis: Default::default(),
            shared_child: Default::default(),
            slot: Default::default(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[metrics]
#[derive(Default)]
struct ResponseData {
    counter: Counter,
}

#[metrics]
#[derive(Default)]
struct MoreResponseData {
    a_counter: Counter,

    max_response_duration: MaxDuration,
}

// You can build your own concurrent fields that
// have opinionated handling via your own `CloseValue` impl.
// This one will implicitly add the duration for the final background
// task to complete.
#[derive(Debug)]
struct MaxDuration(Mutex<Instant>);
impl Default for MaxDuration {
    fn default() -> Self {
        Self(Mutex::new(Instant::now()))
    }
}
impl CloseValue for MaxDuration {
    // for mutexes, you have to decide what to do if a task
    // holding them panics, in this case we drop the metric
    type Closed = Option<Duration>;

    fn close(self) -> Self::Closed {
        self.0.lock().ok().map(|start| start.elapsed())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to(std::io::stdout()),
    );
    handle_request("a").await;
    handle_request("b").await;
}

async fn handle_request(request_id: &str) {
    let mut metrics = RequestMetrics::init(request_id);

    // If you have a child metric you need to send owned access to a background task,
    // and only need &self to update it, and know all tasks will complete (or else drop the metric fields),
    // SharedChild is cheapest. Just remember that metric will be eaten if not all tasks finish.
    for i in 0..10 {
        let child_metrics = metrics.shared_child.clone();
        let duration_millis = metrics
            .max_response_duration_before_foreground_closes_millis
            .clone();
        tokio::task::spawn(async move {
            let start = Instant::now();
            tokio::time::sleep(Duration::from_millis(i)).await;
            child_metrics.counter.increment();
            // even if the shared child metrics are dropped due to foreground finishing first,
            // this counter will flush with the value set to the last one that finished prior to foreground flushed
            duration_millis.set(start.elapsed().as_millis().try_into().unwrap());
        });
    }
    // this will result in metrics from #3-10 being ignored, you could use a JoinSet to work around this
    // if you are willing to wait in foreground
    tokio::time::sleep(Duration::from_millis(2)).await;
    let flush_guard = metrics.flush_guard();

    // If you want to instead wait to flush the parent until all background tasks finish, without awaiting
    // in foreground, you can use a Slot with SlotGuard::delay_on() and Arc the SlotGuard
    let slot_guard = metrics
        .slot
        // see `./configuring-flush-delay-time.rs` for more on this api such as adding a timeout
        .open(metrique::OnParentDrop::Wait(flush_guard))
        .expect("slot has not been opened yet");
    // we need to arc the handle to share it around, and then the final one dropping will flush both itself and the parent.
    let shareable_slot_guard = Arc::from(slot_guard);
    for i in 0..10 {
        let metrics = shareable_slot_guard.clone();
        tokio::task::spawn(async move {
            tokio::time::sleep(Duration::from_millis(i)).await;
            metrics.a_counter.increment();
            // our custom MaximumDuration will also implicitly be set to the time from request
        });
    }

    // If you need to pass the parent metric entry around to multiple places (e.g. seprate Tokio tasks), you can create a handle.
    // Handles can only mutate fields that can be mutated through a shared reference, (e.g. atomics).
    let handle = metrics.handle();

    for _i in 0..10 {
        // you can clone the entire handle and call shared methods on it
        let handle = handle.clone();
        tokio::task::spawn(async move {
            handle.counter.increment();
            // you can also access subfields if you have any
        })
        .await
        .unwrap();
    }

    // At the end of this scope (or the final task scope, if we hadn't awaited them), the handle is dropped.
    // IE, wherever the final reference to the handle is held.
    // This calls `close`, flushing all fields.
}
