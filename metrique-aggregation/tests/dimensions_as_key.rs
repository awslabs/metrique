use assert2::check;
use metrique::CloseValue;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::WorkerSink;
use metrique_aggregation::value::{Flatten, Sum};
use metrique_writer::MetricValue;
use metrique_writer::test_util::test_entry_sink;
use metrique_writer::value::WithDimension;
use std::time::Duration;

#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct MyMetrics {
    #[aggregate(strategy = Sum)]
    count: WithDimension<u64>,
}

#[tokio::test]
async fn test_dimensions_as_key() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<MyMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    // Send 30 entries with event=GET
    for _ in 0..30 {
        keyed_sink.send(
            MyMetrics {
                count: 1u64.with_dimension("Event", "GET"),
            }
            .close(),
        );
    }

    // Send 50 entries with event=POST
    for _ in 0..50 {
        keyed_sink.send(
            MyMetrics {
                count: 1u64.with_dimension("Event", "POST"),
            }
            .close(),
        );
    }

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 2);

    // Find the GET entry
    let get_entry = entries
        .iter()
        .find(|e| e.values.get("Event").map(|v| v.as_str()) == Some("GET"))
        .expect("should have GET entry");
    check!(get_entry.metrics["Count"] == 30);

    // Find the POST entry
    let post_entry = entries
        .iter()
        .find(|e| e.values.get("Event").map(|v| v.as_str()) == Some("POST"))
        .expect("should have POST entry");
    check!(post_entry.metrics["Count"] == 50);
}

// Test: Mixed static key + WithDimension field → combined key
#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct MixedKeyMetrics {
    #[aggregate(key)]
    operation: String,
    #[aggregate(strategy = Sum)]
    count: WithDimension<u64>,
}

#[tokio::test]
async fn test_mixed_static_and_dimension_key() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<MixedKeyMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    // Same operation, different dimension → different buckets
    keyed_sink.send(
        MixedKeyMetrics {
            operation: "read".to_string(),
            count: 1u64.with_dimension("Region", "us-east-1"),
        }
        .close(),
    );
    keyed_sink.send(
        MixedKeyMetrics {
            operation: "read".to_string(),
            count: 1u64.with_dimension("Region", "eu-west-1"),
        }
        .close(),
    );
    // Same dimension, different operation → different buckets
    keyed_sink.send(
        MixedKeyMetrics {
            operation: "write".to_string(),
            count: 1u64.with_dimension("Region", "us-east-1"),
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 3);
}

// Test: Multiple WithDimension fields with different classes → combined dimension key
#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct MultiDimMetrics {
    #[aggregate(strategy = Sum)]
    count: WithDimension<u64>,
    #[aggregate(strategy = Sum)]
    bytes: WithDimension<u64>,
}

#[tokio::test]
async fn test_multiple_dimension_fields_different_classes() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<MultiDimMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    keyed_sink.send(
        MultiDimMetrics {
            count: 1u64.with_dimension("Event", "GET"),
            bytes: 100u64.with_dimension("Region", "us-east-1"),
        }
        .close(),
    );
    keyed_sink.send(
        MultiDimMetrics {
            count: 1u64.with_dimension("Event", "GET"),
            bytes: 200u64.with_dimension("Region", "us-east-1"),
        }
        .close(),
    );
    keyed_sink.send(
        MultiDimMetrics {
            count: 1u64.with_dimension("Event", "POST"),
            bytes: 50u64.with_dimension("Region", "us-east-1"),
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    // (GET, us-east-1) and (POST, us-east-1) → 2 buckets
    check!(entries.len() == 2);

    let get_entry = entries
        .iter()
        .find(|e| e.values.get("Event").map(|v| v.as_str()) == Some("GET"))
        .unwrap();
    check!(get_entry.metrics["Count"] == 2);
    check!(get_entry.metrics["Bytes"] == 300);
}

// Test: Two WithDimension fields with identical (class, instance) → dedup, single key
#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct DedupDimMetrics {
    #[aggregate(strategy = Sum)]
    count: WithDimension<u64>,
    #[aggregate(strategy = Sum)]
    errors: WithDimension<u64>,
}

#[tokio::test]
async fn test_dedup_identical_dimensions() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<DedupDimMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    // Both fields have same dimension → should dedup to single key
    keyed_sink.send(
        DedupDimMetrics {
            count: 1u64.with_dimension("Event", "GET"),
            errors: 0u64.with_dimension("Event", "GET"),
        }
        .close(),
    );
    keyed_sink.send(
        DedupDimMetrics {
            count: 1u64.with_dimension("Event", "GET"),
            errors: 1u64.with_dimension("Event", "GET"),
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["Count"] == 2);
    check!(entries[0].metrics["Errors"] == 1);
}

// Test: Empty dimensions → single bucket, no EntryDimensions emitted
#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct EmptyDimMetrics {
    #[aggregate(strategy = Sum)]
    count: WithDimension<u64>,
}

#[tokio::test]
async fn test_empty_dimensions_single_bucket() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<EmptyDimMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    // No dimensions set → all go to same bucket
    keyed_sink.send(
        EmptyDimMetrics {
            count: WithDimension::from(1u64),
        }
        .close(),
    );
    keyed_sink.send(
        EmptyDimMetrics {
            count: WithDimension::from(2u64),
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["Count"] == 3);
}

// Test: Histogram strategy on WithDimension<Duration> → distributions per dimension key
#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct HistDimMetrics {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    latency: WithDimension<Duration>,
}

#[tokio::test]
async fn test_histogram_with_dimension() {
    use metrique_writer::Observation;

    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<HistDimMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    keyed_sink.send(
        HistDimMetrics {
            latency: Duration::from_millis(10).with_dimension("Op", "Read"),
        }
        .close(),
    );
    keyed_sink.send(
        HistDimMetrics {
            latency: Duration::from_millis(20).with_dimension("Op", "Read"),
        }
        .close(),
    );
    keyed_sink.send(
        HistDimMetrics {
            latency: Duration::from_millis(50).with_dimension("Op", "Write"),
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 2);

    let read_entry = entries
        .iter()
        .find(|e| e.values.get("Op").map(|v| v.as_str()) == Some("Read"))
        .unwrap();
    check!(
        read_entry.metrics["Latency"].distribution
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
}

// Test: Flatten with nested struct containing WithDimensions → outer key includes inner dimensions
#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct InnerDimMetrics {
    #[aggregate(strategy = Sum)]
    inner_count: WithDimension<u64>,
}

#[aggregate]
#[metrics(rename_all = "PascalCase")]
pub struct OuterDimMetrics {
    #[aggregate(strategy = Sum)]
    outer_count: WithDimension<u64>,
    #[metrics(flatten)]
    #[aggregate(strategy = Flatten)]
    inner: InnerDimMetrics,
}

#[tokio::test]
async fn test_flatten_with_nested_dimensions() {
    let test_sink = test_entry_sink();
    let keyed_aggregator: KeyedAggregator<OuterDimMetrics> = KeyedAggregator::new(test_sink.sink);
    let keyed_sink = WorkerSink::new(keyed_aggregator, Duration::from_millis(100));

    keyed_sink.send(
        OuterDimMetrics {
            outer_count: 1u64.with_dimension("Region", "us-east-1"),
            inner: InnerDimMetrics {
                inner_count: 10u64.with_dimension("Op", "Read"),
            },
        }
        .close(),
    );
    keyed_sink.send(
        OuterDimMetrics {
            outer_count: 1u64.with_dimension("Region", "us-east-1"),
            inner: InnerDimMetrics {
                inner_count: 20u64.with_dimension("Op", "Read"),
            },
        }
        .close(),
    );
    keyed_sink.send(
        OuterDimMetrics {
            outer_count: 1u64.with_dimension("Region", "us-east-1"),
            inner: InnerDimMetrics {
                inner_count: 5u64.with_dimension("Op", "Write"),
            },
        }
        .close(),
    );

    keyed_sink.flush().await;

    let entries = test_sink.inspector.entries();
    // (us-east-1, Read) and (us-east-1, Write) → 2 buckets
    check!(entries.len() == 2);
}
