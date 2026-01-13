use assert2::check;
use metrique::CloseValue;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::sink::WorkerSink;
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
async fn test_keyed_sink() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<ApiCall> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    // Send multiple calls to api1
    keyed_sink.send(
        ApiCall {
            endpoint: "api1".to_string(),
            latency: Duration::from_millis(10),
        }
        .close(),
    );
    keyed_sink.send(
        ApiCall {
            endpoint: "api1".to_string(),
            latency: Duration::from_millis(20),
        }
        .close(),
    );

    // Send one call to api2
    keyed_sink.send(
        ApiCall {
            endpoint: "api2".to_string(),
            latency: Duration::from_millis(50),
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    eprintln!("Entries: {:#?}", entries);
    check!(entries.len() == 2);

    use metrique_writer::Observation;

    // Find the api1 entry
    let api1 = entries
        .iter()
        .find(|e| e.values.get("endpoint").unwrap().as_str() == "api1")
        .unwrap();
    let api1_latency = &api1.metrics["latency"];
    check!(
        api1_latency.distribution
            == [
                Observation::Repeated {
                    total: 10.0,
                    occurrences: 1
                },
                Observation::Repeated {
                    total: 20.0,
                    occurrences: 1
                }
            ]
    );

    // Find the api2 entry
    let api2 = entries
        .iter()
        .find(|e| e.values.get("endpoint").unwrap().as_str() == "api2")
        .unwrap();
    let api2_latency = &api2.metrics["latency"];
    check!(
        api2_latency.distribution
            == [Observation::Repeated {
                total: 50.0,
                occurrences: 1
            }]
    );
}
