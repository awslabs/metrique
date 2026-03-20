// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique_writer::{
    AnyEntrySink, Entry, EntryConfig, EntrySink, EntryWriter, Observation,
    test_util::{TestEntry, TestEntrySink, render_entry_sink, test_entry_sink, to_test_entry},
    value::Distribution,
};
use metrique_writer_format_emf::Emf;

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
    assert_eq!(handle.entries()[0].metrics["a"], 1);
    assert_eq!(handle.entries()[0].metrics["a"].as_bool(), true);
    assert_eq!(handle.entries()[0].metrics["a"], true);
    assert!(handle.entries()[0].metrics["a"] > 0);

    assert_eq!(handle.entries()[0].metrics["a"], 1.0);
    assert!(handle.entries()[0].metrics["a"] > 0.0);
    assert_eq!(handle.entries()[0].metrics["b"], 2.5);
    assert_eq!(handle.entries()[0].metrics["b"], 2);
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

#[test]
fn render_queue_captures_emf_output() {
    #[derive(Entry)]
    #[entry(rename_all = "PascalCase")]
    struct MyMetrics {
        request_count: u64,
    }

    let (queue, sink) = render_entry_sink(Emf::all_validations("MyNamespace".into(), vec![vec![]]));

    // entries() returns empty vec before first entry
    assert!(queue.entries().is_empty());

    sink.append(MyMetrics { request_count: 7 });

    let entries = queue.entries();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("\"MyNamespace\""));
    assert!(entries[0].contains("\"RequestCount\""));

    // multiple appends
    sink.append(MyMetrics { request_count: 42 });
    sink.append(MyMetrics { request_count: 99 });

    let entries = queue.entries();
    assert_eq!(entries.len(), 3);
    assert!(entries[1].contains("\"RequestCount\""));
    assert!(entries[2].contains("\"RequestCount\""));

    // smoke test of the Display impl
    let display = queue.to_string();
    assert!(!display.is_empty());
    assert_eq!(
        display.matches("\"MyNamespace\"").count(),
        3,
        "each appended entry should appear in the display"
    );
}
