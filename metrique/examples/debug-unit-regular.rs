use metrique::unit_of_work::metrics;
use metrique::unit::Megabyte;

#[metrics]
struct TestMetrics {
    operation: &'static str,
    
    #[metrics(unit = Megabyte)]
    size: u64,
}

fn main() {
    println!("Testing regular unit handling...");
}
