//! Integration test demonstrating manual aggregation without proc macros.
//!
//! This test shows how the aggregation traits work together by manually implementing
//! all the required traits for a realistic metrics scenario.

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate::{
    AggregatableEntry, AggregateValue, Aggregated, AggregatedEntry, FromKey,
};
use metrique_aggregation::{Counter, histogram::Histogram};
use std::time::Duration;

/// A request metric that tracks operation, status, count, and latency.
#[derive(Clone)]
#[metrics]
struct RequestMetrics {
    operation: &'static str,
    status_code: u16,
    request_count: u64,
    latency: Duration,
}

#[metrics]
struct ApiCall {
    latency: Duration,
    number_of_tokens: usize,
}

impl AggregatableEntry for ApiCall {
    type Aggregated = AggregatedApiCall;
}

#[metrics]
#[derive(Default)]
struct AggregatedApiCall {
    latency: <Histogram<Duration> as AggregateValue<Duration>>::Aggregated,
    number_of_tokens: <Counter as AggregateValue<usize>>::Aggregated,
}

impl FromKey<()> for AggregatedApiCall {
    fn new_from_key(_key: ()) -> Self {
        Self::default()
    }
}

impl AggregatedEntry for AggregatedApiCall {
    type Source = ApiCall;

    fn aggregate_into(&mut self, entry: &Self::Source) {
        <Histogram<Duration> as AggregateValue<Duration>>::add_value(
            &mut self.latency,
            &entry.latency,
        );
        <Counter as AggregateValue<usize>>::add_value(
            &mut self.number_of_tokens,
            &entry.number_of_tokens,
        );
    }
}

#[metrics(rename_all = "PascalCase")]
struct ManyRequests {
    #[metrics(flatten)]
    requests: Aggregated<ApiCall>,
}

impl AggregatableEntry for RequestMetrics {
    type Aggregated = AggregatedRequestMetrics;
}

#[metrics]
struct AggregatedRequestMetrics {
    operation: &'static str,
    status_code: u16,
    request_count: <Counter as AggregateValue<u64>>::Aggregated,
    latency: <Histogram<Duration> as AggregateValue<Duration>>::Aggregated,
}

impl AggregatedEntry for AggregatedRequestMetrics {
    type Source = RequestMetrics;

    fn aggregate_into(&mut self, entry: &Self::Source) {
        <Counter as AggregateValue<u64>>::add_value(&mut self.request_count, &entry.request_count);
        <Histogram<Duration> as AggregateValue<Duration>>::add_value(
            &mut self.latency,
            &entry.latency,
        );
    }
}

#[test]
fn test_many_requests() {
    use metrique::test_util::{TestEntrySink, test_entry_sink};

    let mut metrics = ManyRequests {
        requests: Aggregated::default(),
    };

    metrics.requests.add(ApiCall {
        latency: Duration::from_millis(100),
        number_of_tokens: 50,
    });

    metrics.requests.add(ApiCall {
        latency: Duration::from_millis(200),
        number_of_tokens: 75,
    });

    metrics.requests.add(ApiCall {
        latency: Duration::from_millis(150),
        number_of_tokens: 60,
    });

    drop(metrics);

    let entries = inspector.entries();
    check!(entries.len() == 1);

    let entry = &entries[0];
    // verify that renames work properly with aggregation
    check!(entry.metrics["Latency"].distribution.len() == 3);
    check!(entry.metrics["NumberOfTokens"].as_u64() == 185);
}
