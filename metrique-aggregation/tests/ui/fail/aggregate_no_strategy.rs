use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;

#[aggregate]
#[metrics]
struct AggregateMe {
    a: usize,
}

fn main() {}
