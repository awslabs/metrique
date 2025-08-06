// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique_writer::{
    AnyEntrySink, Entry, EntryConfig, EntryWriter, Observation,
    test_util::{TestEntry, TestEntrySink, test_entry_sink, to_test_entry},
    value::Distribution,
};

#[test]
fn test_sink_records_entries() {
    // have some config that is ignored, to get coverlay to leave us alone
    #[derive(Debug)]
    struct TestConfig;
    impl EntryConfig for TestConfig {}
    struct TestConfigEntry;
    impl Entry for TestConfigEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.config(&TestConfig);
        }
    }

    #[derive(Entry)]
    struct TestEntry {
        #[entry(flatten)]
        allow_split: TestConfigEntry,
        a: usize,
        b: f64,
        c: &'static str,
    }

    let TestEntrySink {
        inspector: handle,
        sink,
    } = test_entry_sink();
    sink.append_any(TestEntry {
        allow_split: TestConfigEntry,
        a: 1,
        b: 2.5,
        c: "label",
    });

    assert_eq!(handle.entries().len(), 1);
    // check coercions & auto equality & auto ord
    assert_eq!(handle.entries()[0].metrics["a"].as_u64(), 1);
    assert_eq!(handle.entries()[0].metrics["a"].as_bool(), true);
    assert_eq!(handle.entries()[0].metrics["a"], true);
    assert_eq!(handle.entries()[0].metrics["a"], 1);
    assert!(handle.entries()[0].metrics["a"] > 0);

    assert_eq!(handle.entries()[0].metrics["a"].as_f64(), 1.0);
    assert_eq!(handle.entries()[0].metrics["a"], 1.0);
    assert!(handle.entries()[0].metrics["a"] > 0.0);
    assert_eq!(handle.entries()[0].metrics["b"].as_f64(), 2.5);
    assert_eq!(handle.entries()[0].metrics["b"].as_u64(), 2);
    assert_eq!(handle.entries()[0].values["c"], "label");
}

fn entry_with_repeat() -> TestEntry {
    #[derive(Entry)]
    struct Test {
        a: Distribution<Observation, 1>,
    }
    to_test_entry(Test {
        a: [Observation::Repeated {
            total: 123.0,
            occurrences: 4,
        }]
        .into_iter()
        .collect(),
    })
}

#[test]
#[should_panic(expected = "found a repeated sample")]
fn repeated_entry_errors_u64() {
    let _panics = entry_with_repeat().metrics["a"].as_u64();
}

#[test]
#[should_panic(expected = "found a repeated sample")]
fn repeated_entry_errors_f64() {
    let _panics = entry_with_repeat().metrics["a"].as_f64();
}
