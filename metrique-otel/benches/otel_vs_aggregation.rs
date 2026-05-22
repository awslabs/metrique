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
//! - `drain_after_burst[(threads, per_thread)]`: answers "is the worker
//!   keeping up?". Producers push `per_thread` entries each across
//!   `threads` threads, then we time the worker's drain (an explicit
//!   `WorkerSink::flush`). Drain throughput = `threads * per_thread / drain_time`,
//!   which is the worker's effective processing rate. If that rate stays
//!   roughly flat across workloads, the worker is keeping up; if it
//!   collapses, the worker is the bottleneck.

use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use divan::{Bencher, black_box};
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_otel::aggregate::{OtelCounter, OtelGauge, OtelHistogram, OtelUpDownCounter};
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Gauge, Histogram, UpDownCounter};
use metrique_writer_core::{Entry, entry::EntryWriter, sink::EntrySink, value::ForceFlag};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

const CARDINALITIES: &[usize] = &[1, 10, 100, 1000];

/// (producer threads, entries per thread); total burst is the product.
const DRAIN_WORKLOADS: &[(usize, usize)] = &[
    (1, 10_000),
    (4, 10_000),
    (8, 10_000),
    (1, 100_000),
    (4, 100_000),
    (8, 100_000),
];

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
// Drain: does the worker keep up?
// ---------------------------------------------------------------------------

/// Producers push a fixed burst (`threads * per_thread` entries) and then stop;
/// we time the worker's drain via an explicit `flush`. Producer wall-clock is
/// excluded; only the drain after the burst is in the measurement. Dividing
/// burst size by drain time yields the worker's effective processing rate.
#[divan::bench(args = DRAIN_WORKLOADS)]
fn drain_after_burst(bencher: Bencher, workload: (usize, usize)) {
    let (threads, per_thread) = workload;
    let total = threads * per_thread;

    // One tokio runtime for the whole bench; flush is async.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");

    // Pre-leak one (op, region) per producer thread, once.
    let keys: Vec<(&'static str, &'static str)> = (0..threads)
        .map(|t| {
            let op: &'static str = Box::leak(format!("Op_{t}").into_boxed_str());
            let region: &'static str = Box::leak(format!("Region_{t}").into_boxed_str());
            (op, region)
        })
        .collect();

    bencher
        .counter(total)
        .with_inputs(|| {
            let worker = make_aggregation_sink();
            std::thread::scope(|s| {
                for &(op, region) in &keys {
                    let worker = worker.clone();
                    s.spawn(move || {
                        for _ in 0..per_thread {
                            AggMixedEntry::fresh(op, region).close_and_merge(worker.clone());
                        }
                    });
                }
            });
            worker
        })
        .bench_values(|worker| {
            rt.block_on(worker.flush());
        });
}
