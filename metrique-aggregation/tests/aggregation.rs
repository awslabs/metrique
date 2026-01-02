//! Test using the #[aggregate] macro

use assert2::check;
use metrique::timers::Timer;
use metrique::unit::{Byte, Microsecond, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::{AggregateSink, MergeOnDropExt, MutexAggregator};
use metrique_aggregation::traits::Aggregate;
use metrique_aggregation::value::{LastValueWins, MergeOptions, Sum};
use metrique_writer::test_util::{DistributionsExt, test_metric};
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
use std::time::Duration;

#[aggregate]
#[metrics]
#[derive(Clone)]
pub struct ApiCall {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = Sum)]
    #[metrics(unit = Byte)]
    response_size: usize,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCall>,
    request_id: String,
}

#[test]
fn test_macro_aggregation() {
    let mut metrics = RequestMetrics {
        api_calls: Aggregate::default(),
        request_id: "1234".to_string(),
    };

    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
    });
    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
    });

    metrics.api_calls.add(ApiCall {
        latency: Duration::from_millis(200),
        response_size: 75,
    });

    metrics.api_calls.add(ApiCall {
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

#[aggregate(raw)]
#[metrics]
#[derive(Clone)]
struct ApiCallWithEndpoint {
    #[aggregate(key)]
    endpoint: String,
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsWithEndpoint {
    #[metrics(flatten)]
    api_calls: MutexAggregator<ApiCallWithEndpoint>,
    request_id: String,
}

#[test]
fn test_macro_aggregation_with_key() {
    let metrics = RequestMetricsWithEndpoint {
        api_calls: MutexAggregator::with_key(&"GetItem".to_string()),
        request_id: "5678".to_string(),
    };

    ApiCallWithEndpoint {
        endpoint: "GetItem".to_string(),
        latency: Duration::from_millis(50),
    }
    .merge_on_drop(&metrics.api_calls);
    metrics.api_calls.merge(ApiCallWithEndpoint {
        endpoint: "GetItem".to_string(),
        latency: Duration::from_millis(75),
    });

    let entry = test_metric(metrics);
    check!(entry.values["RequestId"] == "5678");
    check!(entry.values["Endpoint"] == "GetItem");
}

#[aggregate]
#[metrics]
struct ApiCallWithTimer {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    #[metrics(name = "latency_2", unit = Microsecond)]
    latency: Timer,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsWithTimer {
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCallWithTimer>,
    request_id: String,
}

#[test]
fn test_original_entry_works_as_expected() {
    let entry = ApiCallWithTimer {
        latency: Timer::start_now(),
    };
    let entry = test_metric(entry);
    check!(entry.metrics.keys().collect::<Vec<_>>() == ["latency_2"]);
}

#[test]
fn test_aggregate_entry_mode_with_timer() {
    let mut metrics = RequestMetricsWithTimer {
        api_calls: Aggregate::default(),
        request_id: "timer-test".to_string(),
    };

    let mut call1 = ApiCallWithTimer {
        latency: Timer::start_now(),
    };
    call1.latency.stop();
    metrics.api_calls.add(call1);

    let mut call2 = ApiCallWithTimer {
        latency: Timer::start_now(),
    };
    call2.latency.stop();
    metrics.api_calls.add(call2);

    let entry = test_metric(metrics);
    check!(entry.metrics["latency_2"].distribution.num_observations() == 2);
    check!(entry.values["RequestId"] == "timer-test");
    check!(entry.metrics["latency_2"].unit == Unit::Second(NegativeScale::Micro));
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsWithTimerMutex {
    #[metrics(flatten)]
    api_calls: MutexAggregator<ApiCallWithTimer>,
    request_id: String,
}

#[test]
fn test_merge_and_close_on_drop() {
    use metrique_aggregation::sink::MergeOnDropExt;

    let metrics = RequestMetricsWithTimerMutex {
        api_calls: MutexAggregator::new(),
        request_id: "merge-close-test".to_string(),
    };

    let mut call = ApiCallWithTimer {
        latency: Timer::start_now(),
    }
    .merge_and_close_on_drop(&metrics.api_calls);
    std::thread::sleep(Duration::from_millis(10));
    call.latency.stop();
    drop(call);

    let entry = test_metric(metrics);
    check!(entry.metrics["latency_2"].distribution.len() == 1);
    check!(entry.values["RequestId"] == "merge-close-test");
}

#[test]
fn last_value_wins() {
    #[aggregate(owned)]
    #[metrics]
    struct MetricWithOwnedValue {
        #[aggregate(strategy = MergeOptions<LastValueWins>)]
        value: Option<String>,
    }
}
