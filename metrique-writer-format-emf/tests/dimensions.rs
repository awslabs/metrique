// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique_writer::{
    Entry, EntryConfig, EntryIoStream, EntryIoStreamExt as _, EntryWriter, FormatExt, MetricFlags,
    Observation, Unit, ValidationError, Value, ValueWriter, entry::WithGlobalDimensions,
};
use metrique_writer_format_emf::{AllowSplitEntries, Emf};
use smallvec::SmallVec;
use std::{
    borrow::Cow,
    collections::HashSet,
    time::{Duration, SystemTime},
};

pub(crate) type CowStr = std::borrow::Cow<'static, str>;

pub struct MyEntryWriter(Vec<(String, String)>);
impl<'a> EntryWriter<'a> for MyEntryWriter {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.0.push((
            "timestamp".to_string(),
            format!(
                "{}",
                timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64()
            ),
        ));
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        value.write(MyValueWriter(self, name.into()));
    }

    fn config(&mut self, _config: &dyn EntryConfig) {}
}

pub struct MyValueWriter<'a>(&'a mut MyEntryWriter, Cow<'a, str>);
impl ValueWriter for MyValueWriter<'_> {
    fn string(self, value: &str) {
        self.0.0.push((self.1.to_string(), value.to_string()));
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        self.0.0.push((
            self.1.to_string(),
            format!(
                "{:?} {:?} {:?}",
                distribution.into_iter().collect::<Vec<_>>(),
                unit,
                dimensions.into_iter().collect::<Vec<_>>()
            ),
        ));
    }

    fn error(self, error: ValidationError) {
        panic!("{error}");
    }
}

#[test]
fn test_global_dimensions_denylist() {
    struct TestEntry;
    impl Entry for TestEntry {
        fn write<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {
            panic!("Not to be called in this test!");
        }
    }

    let global_dimensions: SmallVec<[(CowStr, CowStr); 1]> = SmallVec::with_capacity(1);
    let mut global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
    global_dimensions_denylist.insert("NonDimMetric".into());
    let mut global_dimensions_entry = WithGlobalDimensions::<_, 1>::new_with_global_dimensions(
        TestEntry,
        global_dimensions,
        global_dimensions_denylist,
    );

    let converted_global_dimensions_denylist = global_dimensions_entry.global_dimensions_denylist();
    assert!(converted_global_dimensions_denylist.contains("NonDimMetric"));

    let empty_global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
    global_dimensions_entry.clear_global_dimensions_denylist();
    assert_eq!(
        *global_dimensions_entry.global_dimensions_denylist(),
        empty_global_dimensions_denylist
    );
}

#[test]
fn test_adds_global_dimensions_emf() {
    struct TestEntry;
    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.config(const { &AllowSplitEntries::new() });
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(12345.6789));
            writer.value("Time", &Duration::from_millis(42));
            writer.value("Operation", "Foo");
            writer.value("BasicIntCount", &1234u64);
            writer.value("NonDimMetric", &1235u64);
        }
    }

    let mut global_dimensions: SmallVec<[(CowStr, CowStr); 1]> = SmallVec::with_capacity(1);
    global_dimensions.push(("AZ".into(), "us-east-1a".into()));
    let mut global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
    global_dimensions_denylist.insert("NonDimMetric".into());
    let global_dimensions_entry = WithGlobalDimensions::<_, 1>::new_with_global_dimensions(
        TestEntry,
        global_dimensions,
        global_dimensions_denylist,
    );

    let mut output = Vec::new();
    let mut stream = Emf::all_validations("MyApp".into(), vec![vec![]]).output_to(&mut output);
    stream.next(&global_dimensions_entry).unwrap();
    stream.flush().unwrap();

    let output = String::from_utf8(output).unwrap();
    let mut output = output.split("\n");
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(output.next().unwrap()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [
                    {
                        "Namespace": "MyApp",
                        "Dimensions": [["AZ"]],
                        "Metrics": [
                            {"Name":"Time","Unit":"Milliseconds"},
                            {"Name":"BasicIntCount"}
                        ]
                    }
                ],
                "Timestamp":12345678
            },
            "AZ": "us-east-1a",
            "Time": 42,
            "BasicIntCount": 1234,
            "Operation": "Foo"
        })
    );
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(output.next().unwrap()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [
                    {
                        "Namespace": "MyApp",
                        "Dimensions": [[]],
                        "Metrics" :[{"Name": "NonDimMetric"}]
                    }
                ],
                "Timestamp": 12345678
            },
            "NonDimMetric": 1235,
            "Operation": "Foo",
        })
    );
    assert_eq!(output.next().unwrap(), "");
    assert!(output.next().is_none());
}

#[test]
fn test_merge_global_dimensions_emf() {
    struct TestEntry;
    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.config(const { &AllowSplitEntries::new() });
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(12345.6789));
            writer.value("Time", &Duration::from_millis(42));
            writer.value("Operation", "Foo");
            writer.value("BasicIntCount", &1234u64);
            writer.value("NonDimMetric", &1235u64);
        }
    }

    let mut global_dimensions: SmallVec<[(CowStr, CowStr); 1]> = SmallVec::with_capacity(1);
    global_dimensions.push(("AZ".into(), "us-east-1a".into()));
    let mut global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
    global_dimensions_denylist.insert("NonDimMetric".into());

    let mut output = Vec::new();
    let mut stream = Emf::all_validations("MyApp".into(), vec![vec![]])
        .output_to(&mut output)
        .merge_global_dimensions(global_dimensions, Some(global_dimensions_denylist));
    stream.next(&TestEntry).unwrap();
    stream.flush().unwrap();

    let output = String::from_utf8(output).unwrap();
    let mut output = output.split("\n");
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(output.next().unwrap()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [
                    {
                        "Namespace": "MyApp",
                        "Dimensions": [["AZ"]],
                        "Metrics": [
                            {"Name":"Time","Unit":"Milliseconds"},
                            {"Name":"BasicIntCount"}
                        ]
                    }
                ],
                "Timestamp":12345678
            },
            "AZ": "us-east-1a",
            "Time": 42,
            "BasicIntCount": 1234,
            "Operation": "Foo"
        })
    );
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(output.next().unwrap()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [
                    {
                        "Namespace": "MyApp",
                        "Dimensions": [[]],
                        "Metrics" :[{"Name": "NonDimMetric"}]
                    }
                ],
                "Timestamp": 12345678
            },
            "NonDimMetric": 1235,
            "Operation": "Foo",
        })
    );
    assert_eq!(output.next().unwrap(), "");
    assert!(output.next().is_none());
}

#[test]
fn test_merge_globals_and_merge_global_dimensions() {
    struct TestEntry;
    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(12345.6789));
            writer.config(const { &AllowSplitEntries::new() });
            writer.value("Time", &Duration::from_millis(42));
            writer.value("Operation", "Foo");
            writer.value("BasicIntCount", &1234u64);
            writer.value("NonDimMetric", &1235u64);
        }
    }

    struct TestGlobals {
        version: String,
    }
    impl Entry for TestGlobals {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Version", &self.version);
        }
    }

    let mut global_dimensions: SmallVec<[(CowStr, CowStr); 2]> = SmallVec::with_capacity(2);
    global_dimensions.push(("AZ".into(), "us-east-1a".into()));
    let mut global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
    global_dimensions_denylist.insert("Time".into());
    global_dimensions_denylist.insert("NonDimMetric".into());

    let mut output = Vec::new();
    let mut stream = Emf::all_validations("MyApp".into(), vec![vec![]])
        .output_to(&mut output)
        .merge_globals(TestGlobals {
            version: "1.0.0".into(),
        })
        .merge_global_dimensions(global_dimensions, Some(global_dimensions_denylist));
    stream.next(&TestEntry).unwrap();
    stream.flush().unwrap();

    let output = String::from_utf8(output).unwrap();
    let mut output: std::str::Split<'_, &'static str> = output.split("\n");
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(output.next().unwrap()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [
                    {
                        "Namespace": "MyApp",
                        "Dimensions": [["AZ"]],
                        "Metrics": [
                            {"Name":"BasicIntCount"}
                        ]
                    }
                ],
                "Timestamp":12345678
            },
            "AZ": "us-east-1a",
            "BasicIntCount": 1234,
            "Version": "1.0.0", // this is *not* included in the per-metric dimensions
            "Operation": "Foo"
        })
    );
    assert_json_diff::assert_json_eq!(
        serde_json::from_str::<serde_json::Value>(output.next().unwrap()).unwrap(),
        serde_json::json!({
            "_aws": {
                "CloudWatchMetrics": [
                    {
                        "Namespace": "MyApp",
                        "Dimensions": [[]],
                        "Metrics" :[{"Name": "Time", "Unit": "Milliseconds"}, {"Name": "NonDimMetric"}]
                    }
                ],
                "Timestamp": 12345678
            },
            "Time": 42,
            "NonDimMetric": 1235,
            "Version": "1.0.0", // this is *not* included in the per-metric dimensions
            "Operation": "Foo",
        })
    );
    assert_eq!(output.next().unwrap(), "");
    assert!(output.next().is_none());
}
