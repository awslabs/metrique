// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::sync::atomic::{
    AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};
use metrique::unit::{Percent, Second};
use metrique::unit_of_work::metrics;
use metrique_writer::sink::VecEntrySink;
use metrique_writer::test_util::{self, TestEntrySink, TestFlag, test_entry_sink};
use metrique_writer::unit::{PositiveScale, UnitTag};
use metrique_writer::value::WithDimensions;
use metrique_writer::{GlobalEntrySink, Unit};
use metrique_writer_core::global_entry_sink;
use std::borrow::Cow;
use std::sync::Arc;

#[derive(metrique_writer::Entry, Default, Clone)]
struct MyEntry {
    foo: u32,
}

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Clone)]
struct Metrics {
    /// A doc comment on _this_ field
    #[metrics(flatten)]
    optional_closed: Option<Nested>,
    #[metrics(flatten_entry, no_close)]
    entry: MyEntry,
}

#[metrics(subfield)]
#[derive(Default, Clone)]
struct MySubfield {
    f: u32,
}

#[metrics]
#[derive(Default, Clone)]
/// A doc comment
struct Nested {
    /// A doc comment on a field
    b: bool,

    c: Option<bool>,

    d: Arc<bool>,
    e: Cow<'static, str>,

    #[metrics(no_close)]
    no_close: ValueWithNoClose,

    #[metrics(flatten)]
    sub: TestFlag<WithDimensions<MySubfield, 1>>,
    // NOTE: currently not possible. Not sure why you'd do this though.
    // box: Box<bool>,
}

#[derive(Clone, Default)]
struct ValueWithNoClose;
impl metrique::__writer::Value for ValueWithNoClose {
    fn write(&self, writer: impl metrique::__writer::ValueWriter) {
        writer.string("no_close");
    }
}

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Clone, Debug)]
struct Units {
    #[metrics(unit = Second)]
    a: usize,

    #[metrics(unit = Percent)]
    b: f64,

    #[metrics(unit = ParsecsPerBit)]
    c: u64,
}

// units can also be defined externally
struct ParsecsPerBit;
impl UnitTag for ParsecsPerBit {
    const UNIT: metrique_writer::Unit = Unit::BitPerSecond(PositiveScale::Giga);
}

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Debug)]
struct Atomics {
    a: AtomicU64,
    b: AtomicU32,
    c: AtomicU16,
    d: AtomicU8,
    e: AtomicUsize,
    f: AtomicBool,
    g: AtomicBool,
}

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Debug)]
struct ArcAtomics {
    a: Arc<AtomicU64>,
    b: Arc<AtomicU32>,
    c: Arc<AtomicU16>,
    d: Arc<AtomicU8>,
    e: Arc<AtomicUsize>,
    f: Arc<AtomicBool>,
}

#[allow(unused)]
struct IAmNotAMetric;

#[metrics(rename_all = "PascalCase")]
struct IgnoredField {
    a: usize,
    #[metrics(ignore)]
    _b: IAmNotAMetric,
}

#[test]
fn flatten_flush_as_expected() {
    let vec_sink = VecEntrySink::new();
    let mut metric = Metrics::default().append_on_drop(vec_sink.clone());
    metric.entry.foo = 1;
    metric.optional_closed = Some(Nested {
        b: true,
        sub: TestFlag::from(WithDimensions::new(MySubfield::default(), "Foo", "Bar")),
        ..Default::default()
    });
    drop(metric);
    let entries = vec_sink.drain();
    let entry = test_util::to_test_entry(&entries[0]);
    // Entry can't be renamed
    assert_eq!(entry.metrics["foo"].as_u64(), 1);
    // but nested can be
    assert_eq!(entry.metrics["B"].as_u64(), 1);
    assert_eq!(entry.metrics["B"].test_flag, false);

    assert_eq!(entry.metrics["F"].as_u64(), 0);
    assert_eq!(
        entry.metrics["F"].dimensions,
        vec![("Foo".to_string(), "Bar".to_string())]
    );
    assert_eq!(entry.metrics["F"].test_flag, true);

    assert_eq!(entry.values["NoClose"], "no_close");
}

#[test]
fn atomics_flush_as_expected() {
    let vec_sink = VecEntrySink::new();
    let atomics = Atomics::default().append_on_drop(vec_sink.clone());
    atomics.a.store(1, Ordering::Relaxed);
    atomics.b.store(2, Ordering::Relaxed);
    atomics.c.store(3, Ordering::Relaxed);
    atomics.d.store(4, Ordering::Relaxed);
    atomics.e.store(5, Ordering::Relaxed);
    atomics.f.store(false, Ordering::Relaxed);
    atomics.g.store(true, Ordering::Relaxed);
    drop(atomics);
    let entries = vec_sink.drain();
    let entry = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry.metrics["A"].as_u64(), 1);
    assert_eq!(entry.metrics["B"].as_u64(), 2);
    assert_eq!(entry.metrics["C"].as_u64(), 3);
    assert_eq!(entry.metrics["D"].as_u64(), 4);
    assert_eq!(entry.metrics["E"].as_u64(), 5);
    // bools set to 1/0
    assert_eq!(entry.metrics["F"].as_u64(), 0);
    assert_eq!(entry.metrics["G"].as_u64(), 1);
}

#[test]
fn arc_atomics_flush_as_expected() {
    let vec_sink = VecEntrySink::new();
    let atomics = ArcAtomics::default().append_on_drop(vec_sink.clone());
    atomics.a.store(1, Ordering::Relaxed);
    atomics.b.store(2, Ordering::Relaxed);
    atomics.c.store(3, Ordering::Relaxed);
    atomics.d.store(4, Ordering::Relaxed);
    atomics.e.store(5, Ordering::Relaxed);
    atomics.f.store(true, Ordering::Relaxed);
    drop(atomics);

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);

    let entry = test_util::to_test_entry(&entries[0]);

    assert_eq!(entry.metrics["A"].as_u64(), 1);
    assert_eq!(entry.metrics["B"].as_u64(), 2);
    assert_eq!(entry.metrics["C"].as_u64(), 3);
    assert_eq!(entry.metrics["D"].as_u64(), 4);
    assert_eq!(entry.metrics["E"].as_u64(), 5);
    // For boolean values, we need to check if it's 1 (true)
    assert_eq!(entry.metrics["F"].as_u64(), 1);
}

#[test]
fn multiple_sinks() {
    global_entry_sink! { MetricsA };
    global_entry_sink! { MetricsB };
    let TestEntrySink {
        inspector: inspector1,
        sink,
    } = test_entry_sink();
    let _guard1 = MetricsA::set_test_sink(sink);
    let mut e1 = Metrics::default().append_on_drop(MetricsA::sink());
    e1.entry.foo = 10;
    let TestEntrySink {
        inspector: inspector2,
        sink,
    } = test_entry_sink();
    // why would you do this? who knows. but it works.
    let _guard2 = MetricsB::set_test_sink(sink);
    let e2 = Metrics::default().append_on_drop(MetricsB::sink());
    drop(_guard2);

    let mut e3 = Metrics::default().append_on_drop(MetricsA::sink());
    e3.entry.foo = 30;

    drop(e1);
    drop(e2);
    drop(e3);

    // the inner layer captures the inner entry, the outer layer captures entries 1 and 3
    assert_eq!(inspector1.entries().len(), 2);
    assert_eq!(inspector2.entries().len(), 1);
}
