//! Integration test demonstrating manual aggregation without proc macros.
//!
//! This test shows how the aggregation traits work together by manually implementing
//! all the required traits for a realistic metrics scenario.

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate::{
    AggregatableEntry, AggregateValue, Aggregated, AggregatedEntry,
};
//use metrique_aggregation::sink::TypedAggregatingEntrySink;
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
    type Key = ();

    type Aggregated = AggregatedApiCall;

    fn new_aggregated(_key: Self::Key) -> Self::Aggregated {
        AggregatedApiCall {
            latency: Default::default(),
            number_of_tokens: Default::default(),
        }
    }

    fn key(&self) -> Self::Key {
        ()
    }
}

#[metrics]
struct AggregatedApiCall {
    latency: <Histogram<Duration> as AggregateValue<Duration>>::Aggregated,
    number_of_tokens: <Counter as AggregateValue<usize>>::Aggregated,
}

impl AggregatedEntry for AggregatedApiCall {
    type Key = ();
    type Source = ApiCall;

    fn aggregate_into(&mut self, entry: &Self::Source) {
        <Histogram<Duration> as AggregateValue<Duration>>::aggregate(
            &mut self.latency,
            &entry.latency,
        );
        <Counter as AggregateValue<usize>>::aggregate(
            &mut self.number_of_tokens,
            &entry.number_of_tokens,
        );
    }
}

#[metrics]
struct ManyRequests {
    #[metrics(flatten)]
    requests: Aggregated<ApiCall>,
}

impl AggregatableEntry for RequestMetrics {
    type Key = (&'static str, u16);

    type Aggregated = AggregatedRequestMetrics;

    fn new_aggregated(key: Self::Key) -> Self::Aggregated {
        AggregatedRequestMetrics {
            operation: key.0,
            status_code: key.1,
            request_count: Default::default(),
            latency: Default::default(),
        }
    }

    fn key(&self) -> Self::Key {
        (self.operation, self.status_code)
    }
}

#[metrics]
struct AggregatedRequestMetrics {
    operation: &'static str,
    status_code: u16,
    request_count: <Counter as AggregateValue<u64>>::Aggregated,
    latency: <Histogram<Duration> as AggregateValue<Duration>>::Aggregated,
}

impl AggregatedEntry for AggregatedRequestMetrics {
    type Key = (&'static str, u16);

    type Source = RequestMetrics;

    fn aggregate_into(&mut self, entry: &Self::Source) {
        <Counter as AggregateValue<u64>>::aggregate(&mut self.request_count, &entry.request_count);
        <Histogram<Duration> as AggregateValue<Duration>>::aggregate(
            &mut self.latency,
            &entry.latency,
        );
    }
}

#[test]
fn test_many_requests() {
    use metrique::test_util::{test_entry_sink, TestEntrySink};

    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut metrics = ManyRequests {
        requests: Aggregated::default(),
    }.append_on_drop(sink);

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
    check!(entry.metrics["latency"].distribution.len() == 3);
    check!(entry.metrics["number_of_tokens"].as_u64() == 185);
}
