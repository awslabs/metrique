// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Benchmarks for `MetricsPool` type erasure and its two-pass write path.
//!
//! Run: `cargo bench -p metrique-util --bench metrics_pool --features metrics-pool`

use std::borrow::Cow;
use std::time::SystemTime;

use divan::{Bencher, black_box};
use metrique::unit_of_work::metrics;
use metrique::writer::{EntryConfig, EntryWriter, Value};
use metrique::{CloseValue, InflectableEntry, PascalCase};
use metrique_util::MetricsPool;

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

const SIZES: &[usize] = &[1, 4, 16];
const PREFIXES: &[&str] = &[
    "child_00", "child_01", "child_02", "child_03", "child_04", "child_05", "child_06", "child_07",
    "child_08", "child_09", "child_10", "child_11", "child_12", "child_13", "child_14", "child_15",
];

#[metrics]
struct ChildMetrics {
    count: u64,
    operation: &'static str,
}

#[derive(Default)]
struct CountingWriter {
    values: usize,
}

impl<'a> EntryWriter<'a> for CountingWriter {
    fn timestamp(&mut self, _timestamp: SystemTime) {}

    fn value(&mut self, _name: impl Into<Cow<'a, str>>, _value: &(impl Value + ?Sized)) {
        self.values += 1;
    }

    fn config(&mut self, _config: &'a dyn EntryConfig) {}
}

fn populated_pool(entries: usize, collide: bool) -> MetricsPool {
    let pool = MetricsPool::new();
    let base = pool.handle();
    for index in 0..entries {
        let handle = if collide {
            base.clone()
        } else {
            base.with_prefix([PREFIXES[index]])
        };
        handle.append(ChildMetrics {
            count: index as u64,
            operation: "PutObject",
        });
    }
    pool
}

#[divan::bench(args = SIZES)]
fn append_unique(bencher: Bencher, entries: usize) {
    bencher
        .counter(entries)
        .with_inputs(|| {
            let pool = MetricsPool::new();
            let base = pool.handle();
            let handles = (0..entries)
                .map(|index| base.with_prefix([PREFIXES[index]]))
                .collect::<Vec<_>>();
            (pool, handles)
        })
        .bench_values(|(pool, handles)| {
            for (index, handle) in handles.into_iter().enumerate() {
                handle.append(ChildMetrics {
                    count: black_box(index as u64),
                    operation: black_box("PutObject"),
                });
            }
            black_box(pool);
        });
}

#[divan::bench(args = SIZES)]
fn close_and_write_unique(bencher: Bencher, entries: usize) {
    bencher
        .counter(entries)
        .with_inputs(|| populated_pool(entries, false))
        .bench_values(|pool| {
            let closed = pool.close();
            let mut writer = CountingWriter::default();
            InflectableEntry::<PascalCase>::write(&closed, &mut writer);
            black_box(writer.values);
        });
}

#[divan::bench(args = SIZES)]
fn close_and_write_collisions(bencher: Bencher, entries: usize) {
    bencher
        .counter(entries)
        .with_inputs(|| populated_pool(entries, true))
        .bench_values(|pool| {
            let closed = pool.close();
            let mut writer = CountingWriter::default();
            InflectableEntry::<PascalCase>::write(&closed, &mut writer);
            black_box(writer.values);
        });
}

#[divan::bench(args = SIZES)]
fn direct_write_baseline(bencher: Bencher, entries: usize) {
    bencher
        .counter(entries)
        .with_inputs(|| {
            (0..entries)
                .map(|index| {
                    ChildMetrics {
                        count: index as u64,
                        operation: "PutObject",
                    }
                    .close()
                })
                .collect::<Vec<_>>()
        })
        .bench_values(|children| {
            let mut writer = CountingWriter::default();
            for child in &children {
                InflectableEntry::<PascalCase>::write(child, &mut writer);
            }
            black_box(writer.values);
        });
}
