// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique::NoAllocAppendOnDrop;
use metrique::test_util::{TestEntrySink, test_entry_sink};

#[metrics]
#[derive(Default)]
struct Simple {
    value: u32,
    name: &'static str,
}

#[test]
fn drop_appends_entry() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = NoAllocAppendOnDrop::new(
        Simple {
            value: 42,
            name: "test",
        },
        sink,
    );
    guard.value = 99;
    drop(guard);

    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 99);
    check!(entries[0].values["name"] == "test");
}

#[test]
fn discard_does_not_emit() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let guard = NoAllocAppendOnDrop::new(
        Simple {
            value: 1,
            name: "discarded",
        },
        sink,
    );
    guard.discard();

    let entries = inspector.entries();
    check!(entries.is_empty());
}

#[test]
fn emit_appends_and_drop_does_not_double_emit() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = NoAllocAppendOnDrop::new(
        Simple {
            value: 7,
            name: "emitted",
        },
        sink,
    );
    guard.value = 77;
    guard.emit();

    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 77);
}

#[test]
fn deref_mut_mutations_reflected() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = NoAllocAppendOnDrop::new(Simple::default(), sink);
    guard.value = 123;
    guard.name = "mutated";
    drop(guard);

    let entries = inspector.entries();
    check!(entries[0].metrics["value"] == 123);
    check!(entries[0].values["name"] == "mutated");
}

// Compile-time assertion: NoAllocAppendOnDrop is Send when its components are Send
fn _assert_send() {
    fn assert_send<T: Send>() {}
    assert_send::<NoAllocAppendOnDrop<Simple, metrique::DefaultSink>>();
}
