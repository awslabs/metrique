//! Integration test for inline aggregation with Aggregated<T>.

use assert2::check;
use metrique::unit::{Byte, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::{MergeOnDropExt, MutexAggregator};
use metrique_aggregation::traits::{Aggregate, AggregateEntry, AggregateEntryRef};
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
use std::borrow::Cow;
use std::time::Duration;

#[metrics]
#[derive(Clone)]
pub struct ApiCall {
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[metrics(unit = Byte)]
    response_size: usize,

    response_value: Option<String>,
}

#[metrics]
#[derive(Clone)]
struct ApiCallWithEndpoint {
    endpoint: String,

    #[metrics(unit = Millisecond)]
    latency: Duration,
}

impl AggregateEntryRef for ApiCallWithEndpoint {
    fn merge_entry_ref(accum: &mut Self::Aggregated, entry: &Self::Source) {
        accum.latency.add_value(&entry.latency);
    }
}

impl AggregateEntry for ApiCallWithEndpoint {
    type Source = ApiCallWithEndpoint;
    type Aggregated = AggregatedApiCallWithOperation;
    type Key<'a> = Cow<'a, String>;

    fn static_key<'a>(key: Self::Key<'a>) -> Self::Key<'static> {
        Cow::Owned(key.into_owned())
    }

    fn merge_entry(accum: &mut Self::Aggregated, entry: Self::Source) {
        Self::merge_entry_ref(accum, &entry);
    }

    fn new_aggregated<'a>(key: &Self::Key<'a>) -> Self::Aggregated {
        AggregatedApiCallWithOperation {
            endpoint: key.clone().into_owned(),
            latency: Default::default(),
        }
    }

    fn key(source: &Self::Source) -> Self::Key<'_> {
        Cow::Borrowed(&source.endpoint)
    }
}

#[metrics]
struct AggregatedApiCallWithOperation {
    endpoint: String,
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration>,
}

#[metrics]
#[derive(Default)]
pub struct AggregatedApiCall {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration, SortAndMerge>,
    #[metrics(unit = Byte)]
    response_size: usize,
    response_value: Option<String>,
}

impl MergeOnDropExt for ApiCall {}

impl AggregateEntry for ApiCall {
    type Source = Self;
    type Aggregated = AggregatedApiCall;
    type Key<'a> = ();

    fn static_key<'a>(_key: Self::Key<'a>) -> Self::Key<'static> {
        ()
    }

    fn merge_entry(accum: &mut Self::Aggregated, entry: Self::Source) {
        accum.latency.add_value(&entry.latency);
        accum.response_size += entry.response_size;
        accum.response_value = entry.response_value;
    }

    fn new_aggregated<'a>(_key: &Self::Key<'a>) -> Self::Aggregated {
        Self::Aggregated::default()
    }

    fn key(_source: &Self::Source) -> Self::Key<'_> {
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
    api_calls: MutexAggregator<ApiCall>,
    request_id: String,
}

#[test]
fn test_metrics_aggregation_sink() {
    let metrics = RequestMetricsWithSink {
        api_calls: MutexAggregator::new(),
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

    metrics.api_calls.add_raw(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
        response_value: None,
    });
    metrics.api_calls.add_raw(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
        response_value: None,
    });

    metrics.api_calls.add_raw(ApiCall {
        latency: Duration::from_millis(200),
        response_size: 75,
        response_value: None,
    });

    metrics.api_calls.add_raw(ApiCall {
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
