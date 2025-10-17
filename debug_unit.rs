use metrique::unit_of_work::metrics;
use metrique::writer::merge::{Counter, Histogram};
use std::time::Duration;

#[metrics(aggregate)]
struct TestMetrics {
    #[metrics(key)]
    operation: &'static str,
    
    #[metrics(aggregate = Counter)]
    count: u64,
    
    #[metrics(aggregate = Histogram, unit = Megabyte)]
    size: u64,
    
    #[metrics(aggregate = Histogram)]
    latency: Duration,
}

fn main() {
    println!("Testing unit handling...");
}
