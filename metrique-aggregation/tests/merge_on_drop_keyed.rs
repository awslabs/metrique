use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::keyed_sink::KeyedAggregationSink;
use metrique_aggregation::sink::CloseAndMergeOnDropExt;
use metrique_writer::test_util::test_entry_sink;
use std::time::Duration;

#[aggregate]
#[metrics]
pub struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    latency: Duration,
}

#[tokio::test]
async fn test_close_and_merge_on_drop_with_keyed_sink() {
    let test_sink = test_entry_sink();
    let keyed_sink =
        KeyedAggregationSink::<ApiCall>::new(test_sink.sink, Duration::from_millis(100));

    {
        let mut call = ApiCall {
            endpoint: "api1".to_string(),
            latency: Duration::from_millis(10),
        }
        .close_and_merge(keyed_sink.clone());

        // Modify before drop
        call.latency = Duration::from_millis(15);
    } // Automatically merged on drop

    {
        ApiCall {
            endpoint: "api1".to_string(),
            latency: Duration::from_millis(20),
        }
        .close_and_merge(keyed_sink.clone());
    } // Automatically merged on drop

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 1);

    use metrique_writer::Observation;
    let api1 = &entries[0];
    check!(
        api1.metrics["latency"].distribution
            == [
                Observation::Repeated {
                    total: 15.0,
                    occurrences: 1
                },
                Observation::Repeated {
                    total: 20.0,
                    occurrences: 1
                }
            ]
    );
}
