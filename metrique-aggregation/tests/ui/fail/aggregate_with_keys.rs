// This test verifies that you can't insert items with keys into Aggregate<T>.
// Aggregate doesn't do keyed aggregation, so types with keys should use KeyedAggregator instead.

use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, aggregator::Aggregate, value::Sum};

#[aggregate]
#[metrics]
struct WithKeys {
    #[aggregate(key)]
    item_type: String,

    #[aggregate(strategy = Sum)]
    count: u64,
}

fn main() {
    let mut agg = Aggregate::<WithKeys>::default();
    agg.insert(WithKeys {
        item_type: "test".to_string(),
        count: 1,
    });
}
