// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use metrique::emf::Emf;
use metrique::writer::{
    Entry, EntryIoStreamExt, EntrySink, FormatExt, format::Format, sink::BackgroundQueueBuilder,
};
use metrique::{CloseValue, RootEntry, unit_of_work::metrics};
use serde_json::Value;

#[metrics(
        emf::dimension_sets = [
            ["Status", "Operation"],
            ["Operation"],
            []
        ],
        rename_all = "PascalCase",
    )]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(timestamp)]
    timestamp: SystemTime,
    status: &'static str,
    number_of_ducks: usize,
}

#[derive(Entry)]
struct Globals {
    region: &'static str,
}

#[test]
fn test_dimensions_set() {
    // Use all validations so that formatting produces a runtime error
    let mut emf = Emf::all_validations("MyApp".to_string(), vec![vec![]]);
    let mut output = vec![];

    emf.format(
        &RootEntry::new(
            RequestMetrics {
                operation: "operation",
                status: "status",
                timestamp: UNIX_EPOCH,
                number_of_ducks: 1000,
            }
            .close(),
        ),
        &mut output,
    )
    .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert_eq!(
        output,
        r#"{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyApp","Dimensions":[["Status","Operation"],["Operation"],[]],"Metrics":[{"Name":"NumberOfDucks"}]}],"Timestamp":0},"NumberOfDucks":1000,"Operation":"operation","Status":"status"}
"#
    );
}

#[tokio::test]
async fn test_dimensions_merged_with_global_queue() {
    tracing_subscriber::fmt::init();
    let test_sink = metrique_writer_core::test_stream::TestSink::default();
    let (queue, _handle) = BackgroundQueueBuilder::new()
        .flush_interval(Duration::from_micros(1))
        .build(
            Emf::builder("Ns".to_string(), vec![vec!["region".to_string()], vec![]])
                .build()
                .output_to(test_sink.clone())
                .merge_globals(Globals {
                    region: "us-east-1",
                }),
        );
    let entry = RequestMetrics {
        operation: "operation",
        status: "status",
        timestamp: UNIX_EPOCH,
        number_of_ducks: 1000,
    }
    .append_on_drop(queue.clone());
    drop(entry);
    queue.flush_async().await;
    let output = test_sink.dump();
    let output_json: Value = serde_json::from_str(&output).unwrap();
    let dimensions = output_json["_aws"]["CloudWatchMetrics"][0]["Dimensions"].clone();
    let dimensions: Vec<Vec<String>> = serde_json::from_value(dimensions).unwrap();
    assert_eq!(
        dimensions,
        [
            // global dimensions come first
            vec!["region", "Status", "Operation"],
            vec!["region", "Operation"],
            vec!["region"],
            // cartesian join w/ empty dimesion set
            vec!["Status", "Operation"],
            vec!["Operation"],
            vec![]
        ]
    );
}

#[metrics(rename_all = "PascalCase")]
struct VecMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    plugins: Vec<String>,
    request_count: usize,
}

#[test]
fn test_vec_property_emits_json_array_in_emf() {
    let mut emf = Emf::all_validations("App".to_string(), vec![vec![]]);
    let mut output = vec![];

    emf.format(
        &RootEntry::new(
            VecMetrics {
                timestamp: UNIX_EPOCH,
                plugins: vec!["auth".into(), "cache".into()],
                request_count: 5,
            }
            .close(),
        ),
        &mut output,
    )
    .unwrap();

    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["Plugins"], serde_json::json!(["auth", "cache"]));
    assert_eq!(json["RequestCount"], 5);
}

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Operation"]],
    default_flags(metrique::emf::flags::HighStorageResolution),
)]
struct HighResMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    event_count: u64,
    #[metrics(flags(skip(metrique::emf::flags::HighStorageResolution)))]
    low_res_count: u64,
}

#[test]
fn flags_descriptor_with_emf_high_resolution() {
    use metrique::writer::Entry;

    let m = HighResMetrics {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        event_count: 42,
        low_res_count: 7,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];
    let fields: Vec<_> = desc.fields().collect();

    // operation: inherits HighStorageResolutionCtor from default_flags
    assert!(
        fields[0]
            .flags()
            .any(|f| f.is::<metrique::emf::HighStorageResolutionCtor>())
    );

    // event_count: inherits HighStorageResolutionCtor from default_flags
    assert!(
        fields[1]
            .flags()
            .any(|f| f.is::<metrique::emf::HighStorageResolutionCtor>())
    );

    // low_res_count: skip(HighStorageResolutionCtor) suppresses it
    assert!(
        !fields[2]
            .flags()
            .any(|f| f.is::<metrique::emf::HighStorageResolutionCtor>())
    );
}

#[test]
fn flags_write_path_emf_high_resolution() {
    use metrique::writer::format::Format;

    let m = HighResMetrics {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        event_count: 42,
        low_res_count: 7,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Operation".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    // event_count has HighStorageResolution flag -> StorageResolution: 1
    let metrics = &json["_aws"]["CloudWatchMetrics"][0]["Metrics"];
    let event_metric = metrics
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["Name"] == "EventCount")
        .expect("EventCount metric not found");
    assert_eq!(event_metric["StorageResolution"], 1);

    // low_res_count has skip(HighStorageResolution) -> no StorageResolution field
    let low_res_metric = metrics
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["Name"] == "LowResCount")
        .expect("LowResCount metric not found");
    assert!(
        low_res_metric.get("StorageResolution").is_none()
            || low_res_metric["StorageResolution"] == 60
    );
}

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Operation"]],
)]
struct MultiFlagMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    #[metrics(flags(
        metrique::emf::flags::HighStorageResolution,
        metrique::emf::flags::NoMetric
    ))]
    debug_value: u64,
    normal_count: u64,
}

#[test]
fn flags_write_path_multiple_flags_on_field() {
    use metrique::writer::format::Format;

    let m = MultiFlagMetrics {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        debug_value: 99,
        normal_count: 5,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Operation".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    // debug_value has NoMetric flag: value in JSON but NOT in Metrics array
    assert_eq!(json["DebugValue"], 99);
    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    assert!(!metrics.iter().any(|m| m["Name"] == "DebugValue"));

    // normal_count has no flags: appears in Metrics array normally
    assert!(metrics.iter().any(|m| m["Name"] == "NormalCount"));
}

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Op"]],
)]
enum EnumFlagWrite {
    Fast {
        #[metrics(timestamp)]
        ts: SystemTime,
        op: String,
        #[metrics(flags(metrique::emf::flags::HighStorageResolution))]
        latency: u64,
    },
    Slow {
        #[metrics(timestamp)]
        ts: SystemTime,
        op: String,
        latency: u64,
    },
}

#[test]
fn flags_write_path_enum_variant() {
    use metrique::writer::format::Format;

    let fast = EnumFlagWrite::Fast {
        ts: UNIX_EPOCH,
        op: "read".into(),
        latency: 5,
    };
    let closed_fast = metrique::CloseValue::close(fast);
    let entry_fast = metrique::RootEntry::new(closed_fast);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Op".to_string()]]);
    let mut output = vec![];
    emf.format(&entry_fast, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    // Fast variant: latency has HighStorageResolution
    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    let latency_metric = metrics.iter().find(|m| m["Name"] == "Latency").unwrap();
    assert_eq!(latency_metric["StorageResolution"], 1);

    // Slow variant: latency has no flags
    let slow = EnumFlagWrite::Slow {
        ts: UNIX_EPOCH,
        op: "write".into(),
        latency: 100,
    };
    let closed_slow = metrique::CloseValue::close(slow);
    let entry_slow = metrique::RootEntry::new(closed_slow);

    let mut output2 = vec![];
    emf.format(&entry_slow, &mut output2).unwrap();
    let json2: Value = serde_json::from_slice(&output2).unwrap();

    let metrics2 = json2["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    let latency_metric2 = metrics2.iter().find(|m| m["Name"] == "Latency").unwrap();
    // No StorageResolution field (or default 60)
    assert!(
        latency_metric2.get("StorageResolution").is_none()
            || latency_metric2["StorageResolution"] == 60
    );
}
