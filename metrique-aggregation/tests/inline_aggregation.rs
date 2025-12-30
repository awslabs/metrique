//! Integration test for inline aggregation with Aggregated<T>.

use assert2::check;
use metrique::unit::{Byte, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate::{Aggregate, AggregateEntry, AggregateValue};
use metrique_aggregation::counter::{Counter, LastValueWins, MergeOptions};
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::{MergeOnDropExt, MutexSink};
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
use std::borrow::Cow;
use std::time::Duration;

/// INSTRUCTIONS FOR IMPLEMENTING PROC MACRO EXPANSION:
// 0. Testing with this example is your final target. We will initially work with unit tests in `metrique-macro`. Those use
//    snapshot testing so you can easily debug the results of macro expansion.
// 1. When the `aggregate` macro is present, generate the `SourceMetric` implementation and the `Aggregator` struct
//    as present below
// 2. `#[aggregate]` should be defined in the src/metriquq-macro package
// 3. `#[aggregate]` MUST be specified before `#[metrics]`. Check this when expanding `#[metrics]`. If `#[metrics]` sees
//    the aggregate macro, it should return a compilation error: "you must place `#[aggregate]` before #[metrics]
// 4. When expanding the `#[aggregate]` macro, you must strip all `#[aggregate]` annocations! See metrique-macro/src/lib.rs#L1457.
//    You should update that function so we can clean `#[aggregate]` as well
//
// 5. If no fields have `
//
#[metrics]
#[derive(Clone)]
// #[aggregate]
pub struct ApiCall {
    // this argument must be a `Type` -- not string.
    // #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
    // #[aggregate(strategy = Count)]
    #[metrics(unit = Byte)]
    response_size: usize,

    // #[aggregate(strategy = MergeOptions<LastValueWins>)]
    response_value: Option<String>,
}

// #[aggregate]
#[metrics]
#[derive(Clone)]
struct ApiCallWithOperation {
    // #[aggregate(key)]
    endpoint: String,

    // #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

impl AggregateEntry for ApiCallWithOperation {
    type Source = ApiCallWithOperation;

    type Aggregated = AggregatedApiCallWithOperation;

    type Key<'a> = &'a String;

    fn merge_entry<'a>(accum: &mut Self::Aggregated, entry: Cow<'a, Self::Source>) {
        <Histogram<Duration> as AggregateValue<Duration>>::add_value(
            &mut accum.latency,
            &entry.latency,
        );
    }

    fn new_aggregated<'a>(key: Self::Key<'a>) -> Self::Aggregated {
        AggregatedApiCallWithOperation {
            endpoint: key.to_owned(),
            latency: Default::default(),
        }
    }

    fn key<'a>(source: &'a Self::Source) -> Self::Key<'a> {
        &source.endpoint
    }
}

#[metrics]
struct AggregatedApiCallWithOperation {
    endpoint: String,

    #[metrics(unit = Millisecond)]
    latency: <Histogram<Duration> as AggregateValue<Duration>>::Aggregated,
}

// copy all attributes already present on the metrics attribute
#[metrics]
// if no fields are marked with `#[aggregate(key)]`, derive default and impl FromKey<()>
#[derive(Default)]
pub struct AggregatedApiCall {
    // COPY ALL `#[metrics...]` attributes directly
    #[metrics(unit = Millisecond)]
    latency: <Histogram<Duration, SortAndMerge> as AggregateValue<Duration>>::Aggregated,
    #[metrics(unit = Byte)]
    response_size: <Counter as AggregateValue<usize>>::Aggregated,

    response_value: <MergeOptions<LastValueWins> as AggregateValue<Option<String>>>::Aggregated,
}

impl MergeOnDropExt for ApiCall {}

impl AggregateEntry for ApiCall {
    type Source = Self;
    type Aggregated = AggregatedApiCall;
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
        <MergeOptions<LastValueWins> as AggregateValue<Option<String>>>::add_value(
            &mut accum.response_value,
            &entry.response_value,
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
struct RequestMetrics {
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCall>,
    request_id: String,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsWithSink {
    #[metrics(flatten)]
    api_calls: MutexSink<ApiCall>,
    request_id: String,
}

#[test]
fn test_metrics_aggregation_sink() {
    let metrics = RequestMetricsWithSink {
        api_calls: MutexSink::new(),
        request_id: "1234".to_string(),
    };

    let mut metric_item = ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
        response_value: None,
    }
    .merge_on_drop(&metrics.api_calls);
    metric_item.response_value = Some("hello!".to_string());

    drop(metric_item);
    let entry = test_metric(metrics);
    check!(entry.metrics["ResponseSize"].as_u64() == 50);
    check!(entry.metrics["ResponseSize"].unit == Unit::Byte(PositiveScale::One));
    check!(entry.metrics["Latency"].unit == Unit::Second(NegativeScale::Milli));
    check!(entry.values["RequestId"] == "1234");
    check!(entry.values["ResponseValue"] == "hello!");
}

#[test]
fn test_request_metric_aggregation() {
    let mut metrics = RequestMetrics {
        api_calls: Aggregate::default(),
        request_id: "1234".to_string(),
    };

    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
        response_value: None,
    });
    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
        response_value: None,
    });

    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(200),
        response_size: 75,
        response_value: None,
    });

    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(150),
        response_size: 60,
        response_value: None,
    });

    let entry = test_metric(metrics);
    // verify that renames work properly with aggregation
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
