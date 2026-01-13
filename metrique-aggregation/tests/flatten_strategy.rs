use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::{
    aggregate,
    value::{Flatten, Sum},
};
use metrique_writer::test_util::test_metric;

#[aggregate]
#[metrics]
struct InnerStruct {
    #[aggregate(strategy = Sum)]
    count: u64,
}

#[aggregate]
#[metrics]
struct OuterEntry {
    #[metrics(flatten)]
    #[aggregate(strategy = Flatten)]
    inner: InnerStruct,
}

#[test]
fn test_flatten_strategy() {
    // The Flatten strategy works when aggregating multiple AggregateMe entries
    // Each AggregateMe contains an AggregateInner, and Flatten merges them

    use metrique_aggregation::aggregator::Aggregate;

    let mut agg = Aggregate::<OuterEntry>::default();

    agg.insert(OuterEntry {
        inner: InnerStruct { count: 5 },
    });

    agg.insert(OuterEntry {
        inner: InnerStruct { count: 10 },
    });

    let entry = test_metric(agg);

    check!(entry.metrics["count"].as_u64() == 15);
}
