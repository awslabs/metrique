#![deny(unused_must_use)]

use metrique::ServiceMetrics;
use metrique::unit_of_work::metrics;
use metrique::writer::GlobalEntrySink;

#[metrics]
struct MyMetrics {
    operation: &'static str,
}

fn main() {
    MyMetrics {
        operation: "example",
    }
    .append_on_drop(ServiceMetrics::sink());
}
