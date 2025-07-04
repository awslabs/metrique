// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrics::counter;

use metrique_writer_core::test_stream::TestSink;
use metrique_writer_format_emf::Emf;

#[tokio::test]
async fn test_lambda_reporter() {
    let sink = TestSink::default();
    let sink_ = sink.clone();
    metrique_writer::metrics::lambda_reporter::install_reporter_to_writer(
        Emf::no_validations("MyNS".to_string(), vec![vec![]]),
        move || sink_.clone(),
    );
    counter!("my_counter").increment(3);
    counter!("my_counter", "label" => "value1").increment(1);
    counter!("my_counter", "label" => "value2").increment(2);
    metrique_writer::metrics::lambda_reporter::flush_metrics()
        .await
        .unwrap();

    // the metrics emitter iterates over a hashmap so the order is unpredictable
    let dump = sink.dump();
    let mut contents: Vec<_> = dump.split('\n').collect();
    contents.sort();
    let contents = contents.join("\n");

    let timestamp_s = r#""Timestamp":"#;
    let timestamp = &contents[contents.find(timestamp_s).unwrap()..];
    let timestamp = &timestamp[timestamp_s.len()..timestamp.find('}').unwrap()];
    let _timestamp_u64: u64 = timestamp.parse().unwrap(); // check that timestamp is a valid u64
    let expected = r#"
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[["label"]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":$timestamp},"label":"value1","my_counter":1}
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[["label"]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":$timestamp},"label":"value2","my_counter":2}
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[[]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":$timestamp},"my_counter":3}
"#;
    let expected = expected.replace("$timestamp", timestamp);
    assert_eq!(expected.trim(), contents.trim());
}
