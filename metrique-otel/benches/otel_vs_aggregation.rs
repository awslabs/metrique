// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-ingest cost comparison: `OtelSink::append` (direct Otel SDK path) vs
//! the `KeyedAggregator -> WorkerSink -> OtelSink` aggregation pipeline.
//!
//! Run: `cargo bench -p metrique-otel --bench otel_vs_aggregation`
//!
//! ## What this answers
//!
//! "Is the metrique-aggregation backend materially cheaper per `append` than
//! the direct OtelSink path, and how does it behave under key cardinality
//! and thread contention?" The downstream sink is `OtelSink` w/
//! `InMemoryMetricExporter` in both arms, so the comparison is "same end
//! exporter, different path."
//!
//! ## Honesty disclaimer
//!
//! The aggregation arms measure ingest only: channel send on the producer
//! plus an in-place HashMap merge on the worker thread. Downstream OTel
//! export work happens when the aggregator flushes; the bench uses a 1-hour
//! flush interval that never fires, so that cost is excluded by design.
//! That is the *point* of aggregation: N appends collapse into one
//! downstream append per (key) per flush. Read the numbers as "amortizable
//! cost per ingest" (aggregation) vs "full cost per ingest" (direct).
//!
//! ## Benches
//!
//! - `append_otel_direct`: baseline. Identical shape to `format_comparison_otel`
//!   in `otel_vs_emf.rs` — one `OtelSink::append(MixedEntry)` per iteration.
//! - `append_aggregation_single_key`: best case for aggregation. Constant
//!   `(operation, region)` key on every iter, so every merge hits the same
//!   accumulator. Measures `close_and_merge` (drop guard + Sender::send).
//! - `append_aggregation_keyed[cardinality]`: cardinality sweep. Cycles
//!   through N pre-built `(operation, region)` pairs to show HashMap
//!   lookup / resize pressure as the keyspace grows.
//! - `contention_aggregation_single_key[threads]`: N threads, one shared
//!   `WorkerSink`, same key. Highlights whether the single worker thread
//!   becomes a serialization bottleneck. Pair-read against
//!   `contention_same_name` in `otel_append.rs`.
//! - `contention_aggregation_disjoint_keys[threads]`: same shape, each
//!   thread uses its own unique key. Shows whether the worker serializes
//!   regardless of key.

use std::borrow::Cow;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use divan::{Bencher, black_box};
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_otel::aggregate::{OtelCounter, OtelGauge, OtelHistogram, OtelUpDownCounter};
use metrique_otel::{Counter, Gauge, Histogram, OtelSink, UpDownCounter};
use metrique_writer_core::{Entry, entry::EntryWriter, sink::EntrySink, value::ForceFlag};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

const THREADS: &[usize] = &[1, 2, 4, 8];
const CARDINALITIES: &[usize] = &[1, 10, 100, 1000];

// ---------------------------------------------------------------------------
// Baseline entry (same shape as otel_vs_emf.rs::MixedEntry).
// ---------------------------------------------------------------------------

struct MixedEntry {
    requests: ForceFlag<u64, Counter>,
    in_flight: ForceFlag<f64, UpDownCounter>,
    latency_ms: ForceFlag<f64, Histogram>,
    queue_depth: ForceFlag<f64, Gauge>,
    operation: String,
    region: String,
}

impl MixedEntry {
    fn fresh() -> Self {
        Self {
            requests: ForceFlag::from(1u64),
            in_flight: ForceFlag::from(3.0f64),
            latency_ms: ForceFlag::from(12.5f64),
            queue_depth: ForceFlag::from(7.0f64),
            operation: "GetItem".to_owned(),
            region: "us-east-1".to_owned(),
        }
    }
}

impl Entry for MixedEntry {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        w.timestamp(SystemTime::now());
        w.value(Cow::Borrowed("Requests"), &self.requests);
        w.value(Cow::Borrowed("InFlight"), &self.in_flight);
        w.value(Cow::Borrowed("LatencyMs"), &self.latency_ms);
        w.value(Cow::Borrowed("QueueDepth"), &self.queue_depth);
        w.value(Cow::Borrowed("Operation"), &self.operation);
        w.value(Cow::Borrowed("Region"), &self.region);
    }
}

// ---------------------------------------------------------------------------
// Aggregated entry: same field names, declared with aggregation strategies.
// ---------------------------------------------------------------------------

#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct AggMixedEntry {
    #[aggregate(key)]
    operation: String,
    #[aggregate(key)]
    region: String,
    #[aggregate(strategy = OtelCounter)]
    requests: u64,
    #[aggregate(strategy = OtelUpDownCounter)]
    in_flight: f64,
    #[aggregate(strategy = OtelHistogram)]
    latency_ms: f64,
    #[aggregate(strategy = OtelGauge)]
    queue_depth: f64,
}

impl AggMixedEntry {
    fn fresh(operation: &str, region: &str) -> Self {
        Self {
            operation: operation.to_owned(),
            region: region.to_owned(),
            requests: 1,
            in_flight: 3.0,
            latency_ms: 12.5,
            queue_depth: 7.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline construction.
// ---------------------------------------------------------------------------

fn make_otel_sink() -> OtelSink {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter).build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
    OtelSink::builder()
        .with_meter_provider(meter_provider)
        .build()
}

/// `KeyedAggregator -> WorkerSink -> OtelSink`. The 1-hour flush interval
/// keeps the worker thread's downstream OTel work off the hot path; the
/// channel is unbounded so producer-side sends stay O(1). At end of bench
/// the WorkerSink drops, the channel disconnects, and the worker drains
/// before exiting (`worker.rs:66-69`). The `JoinHandle` is detached, so
/// teardown does not block divan's reporting.
fn make_aggregation_sink()
-> WorkerSink<AggMixedEntryEntry, KeyedAggregator<AggMixedEntry, OtelSink>> {
    let otel_sink = make_otel_sink();
    let aggregator = KeyedAggregator::<AggMixedEntry, _>::new(otel_sink);
    WorkerSink::new(aggregator, Duration::from_secs(3600))
}

// ---------------------------------------------------------------------------
// Baseline: direct OtelSink::append.
// ---------------------------------------------------------------------------

#[divan::bench]
fn append_otel_direct(bencher: Bencher) {
    let sink = make_otel_sink();
    // Warm the instrument cache so first-sight cost stays out of timing.
    sink.append(MixedEntry::fresh());
    bencher
        .counter(1usize)
        .with_inputs(MixedEntry::fresh)
        .bench_values(|entry| sink.append(black_box(entry)));
}

// ---------------------------------------------------------------------------
// Aggregation: single key.
// ---------------------------------------------------------------------------

#[divan::bench]
fn append_aggregation_single_key(bencher: Bencher) {
    let worker = make_aggregation_sink();
    // Warm: ensure the worker thread is up and the HashMap has the single
    // accumulator slot for this key.
    AggMixedEntry::fresh("GetItem", "us-east-1").close_and_merge(worker.clone());
    bencher
        .counter(1usize)
        .with_inputs(|| AggMixedEntry::fresh("GetItem", "us-east-1"))
        .bench_values(|entry| {
            entry.close_and_merge(worker.clone());
        });
}

// ---------------------------------------------------------------------------
// Aggregation: cardinality sweep.
// ---------------------------------------------------------------------------

/// Pre-built keyset for the cardinality sweep. Leaks `cardinality` static
/// string pairs so per-iter only does index + `to_owned`, matching the
/// allocations the baseline pays.
fn build_keyset(cardinality: usize) -> Vec<(&'static str, &'static str)> {
    (0..cardinality)
        .map(|i| {
            let op: &'static str = Box::leak(format!("Op_{i}").into_boxed_str());
            let region: &'static str = Box::leak(format!("Region_{i}").into_boxed_str());
            (op, region)
        })
        .collect()
}

#[divan::bench(args = CARDINALITIES)]
fn append_aggregation_keyed(bencher: Bencher, cardinality: usize) {
    let worker = make_aggregation_sink();
    let keys = build_keyset(cardinality);
    // Warm: prime each accumulator slot so the bench measures steady-state
    // hash hits + merges, not first-sight HashMap inserts.
    for (op, region) in &keys {
        AggMixedEntry::fresh(op, region).close_and_merge(worker.clone());
    }
    let cursor = AtomicUsize::new(0);
    let len = keys.len();
    bencher
        .counter(1usize)
        .with_inputs(|| {
            let idx = cursor.fetch_add(1, Ordering::Relaxed) % len;
            let (op, region) = keys[idx];
            AggMixedEntry::fresh(op, region)
        })
        .bench_values(|entry| {
            entry.close_and_merge(worker.clone());
        });
}

// ---------------------------------------------------------------------------
// Aggregation: thread contention.
// ---------------------------------------------------------------------------

#[divan::bench(threads = THREADS)]
fn contention_aggregation_single_key(bencher: Bencher) {
    let worker = make_aggregation_sink();
    // Warm so first-sight cost is excluded.
    AggMixedEntry::fresh("Shared", "us-east-1").close_and_merge(worker.clone());
    bencher.counter(1usize).bench(|| {
        AggMixedEntry::fresh(black_box("Shared"), black_box("us-east-1"))
            .close_and_merge(worker.clone());
    });
}

#[divan::bench(threads = THREADS)]
fn contention_aggregation_disjoint_keys(bencher: Bencher) {
    let worker = make_aggregation_sink();
    let op = thread_op();
    let region = thread_region();
    // Warm one accumulator slot per thread.
    AggMixedEntry::fresh(op, region).close_and_merge(worker.clone());
    bencher.counter(1usize).bench(|| {
        AggMixedEntry::fresh(black_box(op), black_box(region)).close_and_merge(worker.clone());
    });
}

/// Returns a `&'static str` operation name unique to the calling thread.
/// Bounded leak: one small string per thread that ever calls it.
fn thread_op() -> &'static str {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static NAME: Cell<Option<&'static str>> = const { Cell::new(None) };
    }
    NAME.with(|slot| {
        if let Some(s) = slot.get() {
            return s;
        }
        let idx = COUNTER.fetch_add(1, Ordering::Relaxed);
        let leaked: &'static str = Box::leak(format!("Op_{idx}").into_boxed_str());
        slot.set(Some(leaked));
        leaked
    })
}

fn thread_region() -> &'static str {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static NAME: Cell<Option<&'static str>> = const { Cell::new(None) };
    }
    NAME.with(|slot| {
        if let Some(s) = slot.get() {
            return s;
        }
        let idx = COUNTER.fetch_add(1, Ordering::Relaxed);
        let leaked: &'static str = Box::leak(format!("Region_{idx}").into_boxed_str());
        slot.set(Some(leaked));
        leaked
    })
}
