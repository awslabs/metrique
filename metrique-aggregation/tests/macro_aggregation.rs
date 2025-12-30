//! Test using the #[aggregate] macro

use assert2::check;
use metrique::unit::{Byte, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate::{Aggregate, AggregateEntry, AggregateValue};
use metrique_aggregation::aggregate;
use metrique_aggregation::counter::Counter;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
use std::borrow::Cow;
use std::time::Duration;

#[aggregate]
#[metrics]
#[derive(Clone)]
pub struct ApiCallMacro {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
    #[aggregate(strategy = Counter)]
    #[metrics(unit = Byte)]
    response_size: usize,
}

impl AggregateEntry for ApiCallMacro {
    type Source = Self;
    type Aggregated = AggregatedApiCallMacro;
    type Key<'a> = ();

    fn merge_entry<'a>(accum: &mut Self::Aggregated, entry: Cow<'a, Self::Source>) {
        <Histogram<Duration, SortAndMerge> as AggregateValue<Duration>>::add_value(
            &mut accum.latency,
            &entry.latency,
        );
        <Counter as AggregateValue<usize>>::add_value(
            &mut accum.response_size,
            &entry.response_size,
        );
    }

    fn new_aggregated<'a>(_key: Self::Key<'a>) -> Self::Aggregated {
        Self::Aggregated::default()
    }

    fn key<'a>(_source: &'a Self::Source) -> Self::Key<'a> {
        ()
    }
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsMacro {
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCallMacro>,
    request_id: String,
}

#[test]
fn test_macro_aggregation() {
    let mut metrics = RequestMetricsMacro {
        api_calls: Aggregate::default(),
        request_id: "1234".to_string(),
    };

    metrics.api_calls.add(ApiCallMacro {
        latency: Duration::from_millis(100),
        response_size: 50,
    });
    metrics.api_calls.add(ApiCallMacro {
        latency: Duration::from_millis(100),
        response_size: 50,
    });

    metrics.api_calls.add(ApiCallMacro {
        latency: Duration::from_millis(200),
        response_size: 75,
    });

    metrics.api_calls.add(ApiCallMacro {
        latency: Duration::from_millis(150),
        response_size: 60,
    });

    let entry = test_metric(metrics);
    check!(
        entry.metrics["Latency"].distribution
            == vec![
                Observation::Repeated {
                    total: 200.0,
                    occurrences: 2
                },
                Observation::Repeated {
                    total: 150.0,
                    occurrences: 1
                },
                Observation::Repeated {
                    total: 200.0,
                    occurrences: 1
                },
            ]
    );
    check!(entry.metrics["ResponseSize"].as_u64() == 235);
    check!(entry.metrics["ResponseSize"].unit == Unit::Byte(PositiveScale::One));
    check!(entry.metrics["Latency"].unit == Unit::Second(NegativeScale::Milli));
    check!(entry.values["RequestId"] == "1234");
}
