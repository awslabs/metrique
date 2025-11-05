use metrique::{CloseValue, ServiceMetrics};
use metrique::unit_of_work::metrics;
use metrique::writer::{EntrySink, GlobalEntrySink};

#[metrics]
struct MyMetrics {
    operation: &'static str,
}

fn main() {
    let metrics = MyMetrics {
        operation: "example",
    };
    // Missing RootEntry::new around metrics.close()
    ServiceMetrics::sink().append(metrics.close());
}
