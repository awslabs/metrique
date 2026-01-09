//! Example demonstrating manual implementation of the AggregateStrategy traits.

use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::value::ToString;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::keyed_sink::KeyedAggregationSink;
use metrique_aggregation::sink::MergeOnDropExt;
use metrique_aggregation::traits::{AggregateStrategy, Key, Merge};
use metrique_writer::test_util::test_entry_sink;
use std::borrow::Cow;
use std::time::Duration;

#[metrics]
pub struct ApiCall {
    endpoint: String,
    status_code: usize,

    #[metrics(unit = Millisecond)]
    latency: Duration,
}

// Key is a metrics struct
#[derive(Clone, Hash, PartialEq, Eq)]
#[metrics(emf::dimension_sets = [["endpoint", "status_code"]])]
pub struct ApiCallKey<'a> {
    endpoint: Cow<'a, String>,
    #[metrics(format = ToString)]
    status_code: Cow<'a, usize>,
}

// Implement Merge for ApiCall (raw mode - merge the user type directly)
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
pub struct ApiCallByEndpointStatusCode;

impl Key<ApiCall> for ApiCallByEndpointStatusCode {
    type Key<'a> = ApiCallKey<'a>;

    fn from_source(source: &ApiCall) -> Self::Key<'_> {
        ApiCallKey {
            endpoint: Cow::Borrowed(&source.endpoint),
            status_code: Cow::Borrowed(&source.status_code),
        }
    }

    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
        ApiCallKey {
            endpoint: Cow::Owned(key.endpoint.clone().into_owned()),
            status_code: Cow::Owned(key.status_code.clone().into_owned()),
        }
    }
}

const _: () = {
    impl AggregateStrategy for ApiCall {
        type Source = ApiCall;
        type Key = ApiCallByEndpointStatusCode;
    }
};

// For raw mode, we need a raw version of KeyedAggregationSink
// For now, this test demonstrates the trait structure
// In practice, use #[aggregate(raw)] which generates all this automatically

#[tokio::test]
#[ignore] // Ignored because KeyedAggregationSink only supports entry mode currently
async fn test_manual_aggregation_strategy() {
    let test_sink = test_entry_sink();
    let _sink = KeyedAggregationSink::<ApiCall>::new(test_sink.sink, Duration::from_millis(100));

    let api_call = ApiCall {
        endpoint: "GetItem".to_string(),
        latency: Duration::from_millis(10),
        status_code: 200,
    };
    api_call.merge(_sink.clone());

    // TODO: impl trait for keyed aggregation sink
    // let api_call = MergeOnDrop::new(api_call, _sink);

    // This would work if we had KeyedAggregationSinkRaw:
    // sink.send(ApiCall {
    //     endpoint: "GetItem".to_string(),
    //     latency: Duration::from_millis(10),
    //     status_code: 200,
    // });
}
