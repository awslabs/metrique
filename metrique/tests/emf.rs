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
