//! Test using the #[aggregate] macro

use assert2::check;
use metrique::unit::{Byte, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregate::Aggregate;
use metrique_aggregation::counter::Counter;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
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
