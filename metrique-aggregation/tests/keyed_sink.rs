use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::keyed_sink::KeyedAggregationSink;
use metrique_writer::test_util::test_entry_sink;
use std::time::Duration;

#[aggregate(raw)]
#[metrics]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    latency: Duration,
}

#[test]
fn test_keyed_sink() {
    let test_sink = test_entry_sink();
    let keyed_sink =
        KeyedAggregationSink::<ApiCall, _>::new(test_sink.sink, Duration::from_millis(100));

    // Send multiple calls to api1
    keyed_sink.send(ApiCall {
        endpoint: "api1".to_string(),
        latency: Duration::from_millis(10),
    });
    keyed_sink.send(ApiCall {
        endpoint: "api1".to_string(),
        latency: Duration::from_millis(20),
    });

    // Send one call to api2
    keyed_sink.send(ApiCall {
        endpoint: "api2".to_string(),
        latency: Duration::from_millis(50),
    });

    std::thread::sleep(Duration::from_millis(150));

    let entries = test_sink.inspector.entries();
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
