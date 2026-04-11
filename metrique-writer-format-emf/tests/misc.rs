// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use metrique_writer::{
    Entry, EntryIoStream, EntrySink, EntryWriter, FormatExt, sink::BackgroundQueue,
};
use metrique_writer_core::test_stream::TestSink;
use metrique_writer_format_emf::Emf;

struct TestEntry {
    count: u64,
}

impl Entry for TestEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(12345.6789));
        writer.value("Time", &Duration::from_millis(42));
        writer.value("Operation", "Foo");
        writer.value("BasicIntCount", &self.count);
    }
}

#[test]
fn test_output_to_make_writer() {
    let output = Mutex::new(Vec::new());
    let mut stream =
        Emf::all_validations("MyApp".into(), vec![vec![]]).output_to_makewriter(|| {
            let mut output = output.lock().unwrap();
            output.push(TestSink::default());
            output.last_mut().unwrap().clone()
        });
    // create 2 entries to make sure both are recorded
    stream.next(&TestEntry { count: 1 }).unwrap();
    stream.next(&TestEntry { count: 2 }).unwrap();
    stream.flush().unwrap();

    let output = output.into_inner().unwrap();
    assert_eq!(output.len(), 2);
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(&output[0].dump()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [{
                    "Namespace": "MyApp",
                    "Dimensions": [[]],
                    "Metrics": [
                        {"Name": "Time", "Unit": "Milliseconds"},
                        {"Name":"BasicIntCount"}
                    ]
                }],
                "Timestamp": 12345678
            },
            "Time": 42,
            "BasicIntCount": 1,
            "Operation":"Foo"
        })
    );
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(&output[1].dump()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [{
                    "Namespace": "MyApp",
                    "Dimensions": [[]],
                    "Metrics": [
                        {"Name": "Time", "Unit": "Milliseconds"},
                        {"Name":"BasicIntCount"}
                    ]
                }],
                "Timestamp": 12345678
            },
            "Time": 42,
            "BasicIntCount": 2,
            "Operation":"Foo"
        })
    );
}

#[test]
fn test_background_queue_with_invalid_metric() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let output_ = output.clone();
    // this will cause a validation eror because BadDim is not provided
    let stream = Emf::all_validations("MyApp".into(), vec![vec!["BadDim".into()]])
        .output_to_makewriter(move || {
            let mut output = output_.lock().unwrap();
            output.push(TestSink::default());
            output.last_mut().unwrap().clone()
        });
    let (queue, jh) = BackgroundQueue::new(stream);
    queue.append(TestEntry { count: 1 });
    drop(jh);
    let m = output
        .lock()
        .unwrap()
        .iter()
        .map(|m| m.take_string())
        .collect::<Vec<_>>();
    // first entry is an empty entry for the invalid one
    assert_eq!(m[0], "");
    // second entry is a property for the bad entry
    let mut entry = serde_json::from_str::<serde_json::Value>(&m[1]).unwrap();
    entry["_aws"]["Timestamp"] = 0.into();
    assert_json_diff::assert_json_eq!(
        entry,
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [{
                    "Namespace": "MyApp",
                    "Dimensions": [["BadDim"]],
                    "Metrics": []
                }],
                "Timestamp": 0
            },
            "MetriqueValidationError":
                "metric entry could not be formatted correctly, call tracing_subscriber::fmt::init to see more detailed information"
        })
    );
}

struct VecEntry {
    plugins: Vec<String>,
}

impl Entry for VecEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        writer.value("Plugins", &self.plugins);
    }
}

#[test]
fn test_vec_emits_json_array_in_emf() {
    let sink = TestSink::default();
    let mut stream = Emf::all_validations("App".into(), vec![vec![]]).output_to(sink.clone());
    stream
        .next(&VecEntry {
            plugins: vec!["auth".into(), "logging".into(), "cache".into()],
        })
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&sink.dump()).unwrap();
    assert_json_diff::assert_json_eq!(
        output["Plugins"],
        serde_json::json!(["auth", "logging", "cache"])
    );
}

#[test]
fn test_empty_vec_emits_empty_array_in_emf() {
    let sink = TestSink::default();
    let mut stream = Emf::all_validations("App".into(), vec![vec![]]).output_to(sink.clone());
    stream.next(&VecEntry { plugins: vec![] }).unwrap();
    let output: serde_json::Value = serde_json::from_str(&sink.dump()).unwrap();
    assert_json_diff::assert_json_eq!(output["Plugins"], serde_json::json!([]));
}

struct VecOptionEntry {
    tags: Vec<Option<String>>,
}

impl Entry for VecOptionEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        writer.value("Tags", &self.tags);
    }
}

#[test]
fn test_vec_with_none_elements_skips_them_in_emf() {
    let sink = TestSink::default();
    let mut stream = Emf::all_validations("App".into(), vec![vec![]]).output_to(sink.clone());
    stream
        .next(&VecOptionEntry {
            tags: vec![Some("a".into()), None, Some("c".into())],
        })
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&sink.dump()).unwrap();
    assert_json_diff::assert_json_eq!(output["Tags"], serde_json::json!(["a", "c"]));
}

#[test]
fn test_single_element_vec_in_emf() {
    let sink = TestSink::default();
    let mut stream = Emf::all_validations("App".into(), vec![vec![]]).output_to(sink.clone());
    stream
        .next(&VecEntry {
            plugins: vec!["only".into()],
        })
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&sink.dump()).unwrap();
    assert_json_diff::assert_json_eq!(output["Plugins"], serde_json::json!(["only"]));
}
