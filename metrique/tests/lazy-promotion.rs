// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use assert2::check;
use metrique::test_util::{TestEntrySink, test_entry_sink};
use metrique::unit_of_work::metrics;

#[metrics]
#[derive(Default)]
struct Simple {
    value: u32,
    name: &'static str,
}

#[test]
fn drop_without_promotion_emits_once() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 42,
        name: "hello",
    }
    .append_on_drop(sink);
    guard.value = 99;
    drop(guard);

    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 99);
    check!(entries[0].values["name"] == "hello");
}

#[test]
fn discard_without_promotion_does_not_emit() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let guard = Simple {
        value: 1,
        name: "discarded",
    }
    .append_on_drop(sink);
    guard.discard();

    let entries = inspector.entries();
    check!(entries.is_empty());
}

#[test]
fn discard_after_flush_guard_does_not_emit() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let guard = Simple::default().append_on_drop(sink);
    let flush = guard.flush_guard();
    guard.discard();
    drop(flush);

    let entries = inspector.entries();
    check!(entries.is_empty());
}

#[test]
fn flush_guard_then_drop_parent_then_drop_guard() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 10,
        name: "deferred",
    }
    .append_on_drop(sink);
    guard.value = 20;

    let flush = guard.flush_guard();

    drop(guard);
    // Not yet emitted — flush guard alive
    check!(inspector.entries().is_empty());

    drop(flush);
    // Now emitted
    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 20);
}

#[test]
fn flush_guard_dropped_before_parent() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 5,
        name: "immediate",
    }
    .append_on_drop(sink);
    guard.value = 55;

    let flush = guard.flush_guard();
    drop(flush);

    // Flush guard gone, but parent still alive — no emission yet
    check!(inspector.entries().is_empty());

    drop(guard);
    // Now emitted at parent drop
    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 55);
}

#[test]
fn multiple_flush_guards_all_must_drop() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let guard = Simple::default().append_on_drop(sink);

    let f1 = guard.flush_guard();
    let f2 = guard.flush_guard();
    let f3 = guard.flush_guard();

    drop(guard);
    check!(inspector.entries().is_empty());

    drop(f1);
    check!(inspector.entries().is_empty());

    drop(f2);
    check!(inspector.entries().is_empty());

    drop(f3);
    check!(inspector.entries().len() == 1);
}

#[test]
fn force_flush_guard_dropped_before_parent() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 7,
        name: "force",
    }
    .append_on_drop(sink);
    guard.value = 77;

    let force = guard.force_flush_guard();
    drop(force);
    // Force flush guard gone, parent still alive
    check!(inspector.entries().is_empty());

    drop(guard);
    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 77);
}

#[test]
fn force_flush_overrides_flush_guard() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let guard = Simple::default().append_on_drop(sink);

    let flush = guard.flush_guard();
    let force = guard.force_flush_guard();

    drop(guard);
    // Flush guard still alive, but force flush will override
    check!(inspector.entries().is_empty());

    drop(force);
    // Force flush dropped — emits regardless of flush guard
    check!(inspector.entries().len() == 1);

    drop(flush);
    // No double-emit
    check!(inspector.entries().len() == 1);
}

#[test]
fn handle_without_promotion_emits_on_last_drop() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 3,
        name: "handled",
    }
    .append_on_drop(sink);
    guard.value = 33;

    let handle = guard.handle();
    check!(inspector.entries().is_empty());

    let h2 = handle.clone();
    drop(handle);
    check!(inspector.entries().is_empty());

    drop(h2);
    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 33);
}

#[test]
fn deref_mut_mutations_reflected() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple::default().append_on_drop(sink);
    guard.value = 123;
    guard.name = "mutated";
    drop(guard);

    let entries = inspector.entries();
    check!(entries[0].metrics["value"] == 123);
    check!(entries[0].values["name"] == "mutated");
}

#[test]
fn flush_guard_then_handle_emits_on_last_guard_drop() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 1,
        name: "two-stage",
    }
    .append_on_drop(sink);
    guard.value = 11;

    let flush = guard.flush_guard();
    let handle = guard.handle();

    let h2 = handle.clone();
    drop(handle);
    check!(inspector.entries().is_empty());

    drop(h2);
    // Handle clones all gone, but flush guard still alive
    check!(inspector.entries().is_empty());

    drop(flush);
    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 11);
}

#[test]
fn mutation_after_promotion_reflected_in_emitted_entry() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 1,
        name: "before",
    }
    .append_on_drop(sink);

    let flush = guard.flush_guard();
    // Mutate *after* promotion
    guard.value = 999;
    guard.name = "after";

    drop(guard);
    drop(flush);

    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 999);
    check!(entries[0].values["name"] == "after");
}

#[test]
fn cross_thread_flush_guard_emits_once() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut guard = Simple {
        value: 7,
        name: "threaded",
    }
    .append_on_drop(sink);
    guard.value = 77;

    let flush = guard.flush_guard();
    drop(guard);
    // Not emitted — flush guard alive
    check!(inspector.entries().is_empty());

    // Move flush guard to another thread
    std::thread::spawn(move || drop(flush)).join().unwrap();

    let entries = inspector.entries();
    check!(entries.len() == 1);
    check!(entries[0].metrics["value"] == 77);
}

#[test]
fn discard_after_force_flush_guard_does_not_emit() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let guard = Simple::default().append_on_drop(sink);
    let force = guard.force_flush_guard();
    guard.discard();
    drop(force);

    let entries = inspector.entries();
    check!(entries.is_empty());
}

// Compile-time assertions: use narrow types that actually exercise the impls
fn _assert_send_sync_unpin() {
    use std::marker::PhantomData;
    use std::marker::PhantomPinned;
    use std::sync::MutexGuard;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    fn assert_unpin<T: Unpin>() {}

    // Basic case
    assert_send::<metrique::AppendAndCloseOnDrop<Simple, metrique::DefaultSink>>();
    assert_sync::<metrique::AppendAndCloseOnDrop<Simple, metrique::DefaultSink>>();
    assert_unpin::<metrique::AppendAndCloseOnDrop<Simple, metrique::DefaultSink>>();

    // Sync+!Send entry — exercises the unsafe impl Sync
    #[metrics]
    struct SyncNotSend {
        value: u32,
        #[metrics(ignore)]
        _p: PhantomData<MutexGuard<'static, ()>>,
    }
    assert_sync::<metrique::AppendAndCloseOnDrop<SyncNotSend, metrique::writer::sink::DevNullSink>>(
    );

    // !Unpin entry — exercises the impl Unpin
    #[metrics]
    struct NotUnpin {
        value: u32,
        #[metrics(ignore)]
        _p: PhantomPinned,
    }
    assert_unpin::<metrique::AppendAndCloseOnDrop<NotUnpin, metrique::writer::sink::DevNullSink>>();
}
