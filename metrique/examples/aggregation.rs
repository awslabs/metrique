use metrique::{RootEntry, unit_of_work::metrics};
use metrique_metricsrs::metrics_histogram::Histogram;
use metrique_writer::{EntrySink, sink::BackgroundQueue};
use std::time::Duration;

#[metrics]
struct MyMetrics {
    request_duration: Option<Duration>,
}

struct AggregatingCollector {
    request_duration: Histogram,
    queue: BackgroundQueue<RootEntry<MyMetricsEntry>>,
}

#[allow(deprecated)]
impl EntrySink<RootEntry<MyMetricsEntry>> for AggregatingCollector {
    fn append(&self, entry: RootEntry<MyMetricsEntry>) {
        let metric = entry.metric();
        if let Some(duration) = metric.request_duration {
            self.request_duration.record(duration.as_millis() as u32);
        };
        self.queue.append(entry);
    }

    fn flush_async(&self) -> metrique_writer::sink::FlushWait {
        self.queue.flush_async()
    }
}

impl MyMetrics {
    fn init() -> MyMetricsGuard {
        MyMetrics {
            request_duration: None,
        }
        .append_on_drop(sink)
    }
}

fn main() {}
