// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Benches for the OTel sink's per-`append` hot path.
//!
//! Run: `cargo bench -p metrique-otel --bench otel_append`
//!
//! ## Benches
//!
//! - `append_single_counter`: floor cost of `OtelSink::append` with a 1-field
//!   entry. Output: ns/iter, allocs/iter, items/sec. Regression: any rise in
//!   allocs/iter means a new allocation slipped into the per-entry path.
//! - `append_wide_entry`: sweeps `(metrics, strings)` shapes. Output: ns/iter
//!   per shape, allocs/iter. Regression: per-metric cost growing faster than
//!   the (1, 0) baseline scales points at the N*K attribute fan-out in
//!   `OtelEntryWriter::finish`.
//! - `record_repeated_same_key`: steady-state cache-hit path. Output: ns/iter,
//!   allocs/iter, items/sec. Regression: allocs/iter going up; today it sits
//!   above 0 because `InstrumentKey.name` is allocated on every hit.
//! - `contention_same_name` / `contention_disjoint_names`: N-thread sweep on a
//!   shared sink. Output: ns/iter per thread count. Regression: the gap
//!   between disjoint and same-name growing means cache or instrument-level
//!   contention got worse.

use std::borrow::Cow;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use divan::{Bencher, black_box};
use metrique_otel::OtelSink;
use metrique_otel::flags::Counter;
use metrique_writer_core::{Entry, entry::EntryWriter, sink::EntrySink, value::ForceFlag};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

const THREADS: &[usize] = &[1, 2, 4, 8];
const SHAPES: &[(usize, usize)] = &[(1, 0), (4, 2), (16, 8)];

fn make_sink() -> OtelSink {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter).build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
    OtelSink::builder()
        .with_meter_provider(meter_provider)
        .build()
}

/// Minimal hand-rolled entry: one counter, no strings, no dimensions.
struct CounterEntry {
    name: &'static str,
    value: ForceFlag<u64, Counter>,
}

impl Entry for CounterEntry {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        w.timestamp(SystemTime::now());
        w.value(Cow::Borrowed(self.name), &self.value);
    }
}

/// Entry with `m` counters and `k` string fields. Names are owned strings on
/// the entry so the bench can vary the shape at runtime.
struct WideEntry {
    metrics: Vec<(String, ForceFlag<u64, Counter>)>,
    strings: Vec<(String, String)>,
}

impl WideEntry {
    fn new(metrics: usize, strings: usize) -> Self {
        let metrics = (0..metrics)
            .map(|i| (format!("Metric_{i}"), ForceFlag::from(1u64)))
            .collect();
        let strings = (0..strings)
            .map(|i| (format!("Dim_{i}"), format!("value_{i}")))
            .collect();
        Self { metrics, strings }
    }
}

impl Entry for WideEntry {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        w.timestamp(SystemTime::now());
        for (name, value) in &self.metrics {
            w.value(Cow::Borrowed(name.as_str()), value);
        }
        for (name, value) in &self.strings {
            w.value(Cow::Borrowed(name.as_str()), value);
        }
    }
}

#[divan::bench]
fn append_single_counter(bencher: Bencher) {
    let sink = make_sink();
    bencher.counter(1usize).bench(|| {
        let entry = CounterEntry {
            name: black_box("Counter"),
            value: ForceFlag::from(black_box(1u64)),
        };
        sink.append(entry);
    });
}

#[divan::bench(args = SHAPES)]
fn append_wide_entry(bencher: Bencher, shape: &(usize, usize)) {
    let (metrics, strings) = *shape;
    let sink = make_sink();
    bencher
        .counter(metrics)
        .with_inputs(|| WideEntry::new(metrics, strings))
        .bench_values(|entry| sink.append(entry));
}

#[divan::bench]
fn record_repeated_same_key(bencher: Bencher) {
    let sink = make_sink();
    // Warm the cache so the very first iteration doesn't pay the
    // first-sight instrument-creation cost.
    sink.append(CounterEntry {
        name: "HotKey",
        value: ForceFlag::from(1u64),
    });
    bencher.counter(1usize).bench(|| {
        sink.append(CounterEntry {
            name: black_box("HotKey"),
            value: ForceFlag::from(black_box(1u64)),
        });
    });
}

/// Worst case: every thread hits the same metric name, so they share an
/// instrument and exercise the OTel SDK's per-instrument contention path.
#[divan::bench(threads = THREADS)]
fn contention_same_name(bencher: Bencher) {
    let sink = make_sink();
    sink.append(CounterEntry {
        name: "Shared",
        value: ForceFlag::from(1u64),
    });
    bencher.counter(1usize).bench(|| {
        sink.append(CounterEntry {
            name: black_box("Shared"),
            value: ForceFlag::from(black_box(1u64)),
        });
    });
}

/// Best case: each thread targets a disjoint metric name, so the OTel SDK's
/// per-instrument atomics don't see cross-thread traffic. Any contention left
/// is in the `papaya` cache lookup.
#[divan::bench(threads = THREADS)]
fn contention_disjoint_names(bencher: Bencher) {
    let sink = make_sink();
    let name = thread_name();
    // Warm one entry per thread so first-sight cost stays out of timing.
    sink.append(CounterEntry {
        name,
        value: ForceFlag::from(1u64),
    });
    bencher.counter(1usize).bench(|| {
        sink.append(CounterEntry {
            name: black_box(name),
            value: ForceFlag::from(black_box(1u64)),
        });
    });
}

/// Returns a `&'static str` that is unique to the calling thread for the
/// remainder of the process. Leaks one small string per thread that calls it,
/// bounded by `O(unique threads)`.
fn thread_name() -> &'static str {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static NAME: Cell<Option<&'static str>> = const { Cell::new(None) };
    }
    NAME.with(|slot| {
        if let Some(s) = slot.get() {
            return s;
        }
        let idx = COUNTER.fetch_add(1, Ordering::Relaxed);
        let leaked: &'static str = Box::leak(format!("Metric_{idx}").into_boxed_str());
        slot.set(Some(leaked));
        leaked
    })
}
