//! Approximate tail sampling with `RollingThresholdSink`.
//!
//! Unlike `TopNSink`, this selector retains no full requests. Each window builds
//! a histogram, and the next window forwards requests at or above the previous
//! window's estimated top-N cutoff. That makes selection approximate and one
//! window late, but bounds memory independently of request size and target N.

use std::sync::{Arc, Mutex};

use metrique_aggregation::{
    sink::RollingThresholdSink,
    traits::{AggregateSink, FlushableSink},
};

#[derive(Debug)]
struct Request {
    latency_ms: u64,
}

#[derive(Clone, Default)]
struct CollectingSink(Arc<Mutex<Vec<Request>>>);

impl AggregateSink<Request> for CollectingSink {
    fn merge(&mut self, request: Request) {
        self.0.lock().unwrap().push(request);
    }
}

impl FlushableSink for CollectingSink {
    fn flush(&mut self) {}
}

fn add_window(
    sink: &mut RollingThresholdSink<Request, CollectingSink, impl FnMut(&Request) -> u64>,
    scores: impl IntoIterator<Item = u64>,
) {
    for latency_ms in scores {
        sink.merge(Request { latency_ms });
    }
    sink.flush();
}

fn main() {
    let selected = CollectingSink::default();
    let mut sink =
        RollingThresholdSink::new(10, |request: &Request| request.latency_ms, selected.clone());

    // The first window trains the cutoff and emits no raw requests.
    add_window(&mut sink, 1..=100);
    let threshold = sink.threshold().expect("the first window sets a cutoff");

    // The second window uses the cutoff learned from the first.
    add_window(&mut sink, 1..=100);
    println!(
        "threshold={threshold}ms selected={} of 100 requests",
        selected.0.lock().unwrap().len()
    );
}
