// This test verifies that you can't use Aggregate<T> with a struct that has WithDimensions fields.
// Structs with dimension keys should use KeyedAggregator instead.

use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, aggregator::Aggregate, value::Sum};
use metrique_writer::value::WithDimension;

#[aggregate]
#[metrics]
struct WithDims {
    #[aggregate(strategy = Sum)]
    count: WithDimension<u64>,
}

fn main() {
    let mut agg = Aggregate::<WithDims>::default();
    agg.insert(WithDims {
        count: WithDimension::from(1u64),
    });
}
