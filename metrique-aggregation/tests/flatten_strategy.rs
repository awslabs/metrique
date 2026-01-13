use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, value::{Sum, Flatten}};
use metrique_writer::test_util::test_metric;

#[aggregate]
#[metrics]
struct AggregateInner {
    #[aggregate(strategy = Sum)]
    count: u64,
}

#[aggregate]
#[metrics]
struct AggregateMe {
    #[metrics(flatten)]
    #[aggregate(strategy = Flatten)]
    inner: AggregateInner,
}

#[test]
fn test_flatten_strategy() {
    // The Flatten strategy works when aggregating multiple AggregateMe entries
    // Each AggregateMe contains an AggregateInner, and Flatten merges them
    
    use metrique_aggregation::aggregator::Aggregate as AggregateWrapper;
    
    let mut agg = AggregateWrapper::<AggregateMe>::default();
    
    agg.insert(AggregateMe {
        inner: AggregateInner { count: 5 },
    });
    
    agg.insert(AggregateMe {
        inner: AggregateInner { count: 10 },
    });

    let entry = test_metric(agg);

    check!(entry.metrics["count"].as_u64() == 15);
}
