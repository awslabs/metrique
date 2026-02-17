//! Test using the #[aggregate] macro

use assert2::check;
use metrique::timers::Timer;
use metrique::unit::{Byte, Microsecond, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::Aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::MutexSink;
use metrique_aggregation::value::{KeepLast, Sum};
use metrique_timesource::TimeSource;
use metrique_timesource::fakes::ManuallyAdvancedTimeSource;
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
use std::time::{Duration, UNIX_EPOCH};

#[aggregate]
#[metrics]
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

    metrics.api_calls.insert(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
    });
    metrics.api_calls.insert(ApiCall {
        latency: Duration::from_millis(100),
        response_size: 50,
    });

    metrics.api_calls.insert(ApiCall {
        latency: Duration::from_millis(200),
        response_size: 75,
    });

    metrics.api_calls.insert(ApiCall {
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

#[aggregate(direct)]
#[metrics]
#[derive(Clone)]
struct ApiCallDirect {
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsDirect {
    #[metrics(flatten)]
    api_calls: Aggregate<ApiCallDirect>,
    request_id: String,
}

#[test]
fn test_macro_aggregation_with_multiple_keys() {
    let mut metrics = RequestMetricsDirect {
        api_calls: Aggregate::default(),
        request_id: "9999".to_string(),
    };

    metrics.api_calls.insert_direct(ApiCallDirect {
        latency: Duration::from_millis(30),
    });

    metrics.api_calls.insert_direct(ApiCallDirect {
        latency: Duration::from_millis(45),
    });

    let entry = test_metric(metrics);
    check!(entry.values["RequestId"] == "9999");
}

#[aggregate]
#[metrics]
pub struct ApiCallWithTimer {
    // Using name = "latency_2" to avoid conflicts with other latency fields in this test file
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
    metrics.api_calls.insert(call1);

    let mut call2 = ApiCallWithTimer {
        latency: Timer::start_now(),
    };
    call2.latency.stop();
    metrics.api_calls.insert(call2);

    let entry = test_metric(metrics);
    check!(entry.metrics["latency_2"].num_observations() == 2);
    check!(entry.values["RequestId"] == "timer-test");
    check!(entry.metrics["latency_2"].unit == Unit::Second(NegativeScale::Micro));
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetricsWithTimerMutex {
    #[metrics(flatten)]
    api_calls: MutexSink<Aggregate<ApiCallWithTimer>>,
    request_id: String,
}

#[test]
fn test_merge_and_close_on_drop() {
    let metrics = RequestMetricsWithTimerMutex {
        api_calls: MutexSink::new(Aggregate::default()),
        request_id: "merge-close-test".to_string(),
    };
    let ts = ManuallyAdvancedTimeSource::at_time(UNIX_EPOCH);

    let call = ApiCallWithTimer {
        latency: Timer::start_now_with_timesource(TimeSource::custom(ts.clone())),
    };

    ts.update_instant(Duration::from_secs(10));

    let call = call.close_and_merge(metrics.api_calls.clone());
    drop(call);
    let entry = test_metric(metrics);
    check!(entry.metrics["latency_2"].distribution.len() == 1);
    check!(
        entry.metrics["latency_2"].distribution
            == [Observation::Repeated {
                total: Duration::from_secs(10).as_micros() as f64,
                occurrences: 1
            }]
    );
    check!(entry.values["RequestId"] == "merge-close-test");
}

#[test]
fn test_mutex_sink_close_with_outstanding_references() {
    // This test verifies that MutexSink can be closed even when there are
    // outstanding cloned references (which would cause Arc::try_unwrap to fail)
    let metrics = RequestMetricsWithTimerMutex {
        api_calls: MutexSink::new(Aggregate::default()),
        request_id: "outstanding-ref-test".to_string(),
    };

    // Clone creates an outstanding reference
    let _outstanding_ref = metrics.api_calls.clone();

    // This should not panic - it uses mem::take instead of Arc::try_unwrap
    let entry = test_metric(metrics);
    check!(entry.values["RequestId"] == "outstanding-ref-test");
}

#[test]
fn test_aggregate_with_prefix() {
    #[aggregate]
    #[metrics(prefix = "Api_")]
    pub struct CallMetrics {
        #[aggregate(strategy = Sum)]
        count: u64,

        #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
        #[metrics(unit = Millisecond)]
        duration: Duration,
    }

    #[metrics]
    struct TestMetrics {
        #[metrics(flatten)]
        calls: Aggregate<CallMetrics>,
    }

    let mut metrics = TestMetrics {
        calls: Aggregate::default(),
    };

    metrics.calls.insert(CallMetrics {
        count: 5,
        duration: Duration::from_millis(100),
    });
    metrics.calls.insert(CallMetrics {
        count: 3,
        duration: Duration::from_millis(200),
    });

    let entry = test_metric(metrics);

    // Verify prefix is applied
    check!(entry.metrics["Api_count"].as_u64() == 8);
    check!(entry.metrics["Api_duration"].flatten_and_sort() == vec![100.0, 200.0]);
}

#[test]
fn test_aggregate_histogram_fields() {
    #[aggregate]
    #[metrics]
    pub struct ShardResult {
        #[aggregate(strategy = Sum)]
        rows_scanned: usize,

        #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
        #[metrics(unit = Microsecond)]
        per_row_latency: Histogram<Duration, SortAndMerge>,
    }

    #[metrics(rename_all = "PascalCase")]
    struct QueryMetrics {
        #[metrics(flatten)]
        shards: Aggregate<ShardResult>,
    }

    let mut query = QueryMetrics {
        shards: Aggregate::default(),
    };

    // Shard 1: two observations
    let mut shard1 = ShardResult {
        rows_scanned: 10,
        per_row_latency: Histogram::default(),
    };
    shard1.per_row_latency.add_value(Duration::from_millis(100));
    shard1.per_row_latency.add_value(Duration::from_millis(200));
    query.shards.insert(shard1);

    // Shard 2: one observation
    let mut shard2 = ShardResult {
        rows_scanned: 5,
        per_row_latency: Histogram::default(),
    };
    shard2.per_row_latency.add_value(Duration::from_millis(150));
    query.shards.insert(shard2);

    let entry = test_metric(query);

    // Sum strategy: 10 + 5
    check!(entry.metrics["RowsScanned"].as_u64() == 15);

    // All 3 observations merged into one distribution
    let dist = &entry.metrics["PerRowLatency"];
    check!(dist.num_observations() == 3);
    check!(dist.unit == Unit::Second(NegativeScale::Micro));
    check!(dist.flatten_and_sort() == vec![100_000.0, 150_000.0, 200_000.0]);
}

#[test]
fn test_aggregate_bucketed_histogram_fields() {
    #[aggregate]
    #[metrics]
    pub struct ShardResult {
        #[aggregate(strategy = Histogram<Duration>)]
        #[metrics(unit = Microsecond)]
        latency: Histogram<Duration>,
    }

    #[metrics(rename_all = "PascalCase")]
    struct QueryMetrics {
        #[metrics(flatten)]
        shards: Aggregate<ShardResult>,
    }

    let mut query = QueryMetrics {
        shards: Aggregate::default(),
    };

    // 10 shards, each with 10 observations (100 total)
    for shard in 0..10 {
        let mut result = ShardResult {
            latency: Histogram::default(),
        };
        for i in 0..10 {
            // Values from 1ms to 100ms, uniformly distributed
            let ms = (shard * 10 + i + 1) as u64;
            result.latency.add_value(Duration::from_millis(ms));
        }
        query.shards.insert(result);
    }

    let entry = test_metric(query);
    let dist = &entry.metrics["Latency"];
    check!(dist.num_observations() == 100);
    check!(dist.unit == Unit::Second(NegativeScale::Micro));

    // p50 of 1..=100ms in microseconds = 50_500us
    // Exponential bucketing has ~6.25% error
    let values = dist.flatten_and_sort();
    let p50 = values[values.len() / 2];
    let expected = 50_500.0;
    let error_pct = ((p50 - expected) / expected).abs() * 100.0;
    check!(
        error_pct < 6.25,
        "p50={p50}, expected ~{expected}, error={error_pct}%"
    );
}

#[test]
fn last_value_wins() {
    #[aggregate]
    #[metrics]
    pub struct MetricWithOwnedValue {
        #[aggregate(strategy = KeepLast)]
        value: Option<String>,
    }
}
