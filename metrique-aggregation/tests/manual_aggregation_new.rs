//! Example demonstrating manual implementation of the new AggregateStrategy traits.

use assert2::check;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::keyed_sink::KeyedAggregationSinkNew;
use metrique_aggregation::traits::{AggregateStrategy, Key, Merge};
use metrique_writer::test_util::test_entry_sink;
use std::time::Duration;

#[metrics]
#[derive(Clone)]
pub struct ApiCall {
    endpoint: String,
    #[metrics(unit = Millisecond)]
    latency: Duration,
}

// Key is a metrics struct
#[metrics]
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct ApiCallKey {
    endpoint: String,
}

// Implement Merge for ApiCall
impl Merge for ApiCall {
    type Merged = AggregatedApiCall;
    type MergeConfig = ();

    fn new_merged(_conf: &Self::MergeConfig) -> Self::Merged {
        Self::Merged::default()
    }

    fn merge(accum: &mut Self::Merged, input: Self) {
        accum.latency.add_value(&input.latency);
    }
}

#[metrics]
#[derive(Default)]
pub struct AggregatedApiCall {
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration>,
}

// Key extraction for ApiCall
struct ApiCallKeyExtractor;

impl Key<ApiCall> for ApiCallKeyExtractor {
    type Key<'a> = ApiCallKey;

    fn from_source(source: &ApiCall) -> Self::Key<'_> {
        ApiCallKey {
            endpoint: source.endpoint.clone(),
        }
    }

    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
        key.clone()
    }
}

// Combine into AggregateStrategy
struct ApiCallStrategy;

impl AggregateStrategy for ApiCallStrategy {
    type Source = ApiCall;
    type Key = ApiCallKeyExtractor;
}

#[test]
fn test_new_aggregation_strategy() {
    let test_sink = test_entry_sink();
    let sink = KeyedAggregationSinkNew::<ApiCallStrategy, _>::new(
        test_sink.sink,
        Duration::from_millis(100),
    );

    sink.send(ApiCall {
        endpoint: "GetItem".to_string(),
        latency: Duration::from_millis(10),
    });

    sink.send(ApiCall {
        endpoint: "GetItem".to_string(),
        latency: Duration::from_millis(20),
    });

    sink.send(ApiCall {
        endpoint: "PutItem".to_string(),
        latency: Duration::from_millis(30),
    });

    drop(sink);
    std::thread::sleep(Duration::from_millis(150));

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 2);
}
