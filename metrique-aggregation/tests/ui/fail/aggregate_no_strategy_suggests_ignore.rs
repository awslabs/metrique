use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;

#[aggregate]
#[metrics]
struct AggregateMe {
    #[aggregate(strategy = Sum)]
    count: usize,
    request_id: String,
}

fn main() {}
