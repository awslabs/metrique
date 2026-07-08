// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Like-for-like comparison: identical `Entry` shape, appended to an OTel
//! sink and to an EMF sink.
//!
//! Run: `cargo bench -p metrique-otel --bench otel_vs_emf`
//!
//! ## Benches
//!
//! - `format_comparison_otel`: ns/iter, allocs/iter, items/sec for the OTel
//!   sink. No bytes counter (OTel batches into protobuf later, not at append).
//! - `format_comparison_emf`: ns/iter, allocs/iter, bytes/sec for the EMF
//!   sink (serializing into a discarding `io::Write`). The bytes-per-iter
//!   value is pre-measured once at startup; the figure assumes the
//!   serialized size is stable across iterations (it is, for a fixed
//!   `Entry`).
//!
//! Read together: "OTel costs X ns/append; EMF costs Y ns/append and emits
//! Z bytes/append." Neither number alone is the answer.

use std::borrow::Cow;
use std::io;
use std::time::SystemTime;

use divan::{Bencher, black_box, counter::BytesCount};
use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Gauge, Histogram, UpDownCounter};
use metrique_writer::{EntrySink, format::FormatExt as _, sink::FlushImmediately};
use metrique_writer_core::{Entry, entry::EntryWriter, value::ForceFlag};
use metrique_writer_format_emf::Emf;
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

/// Mixed entry: one of each OTel instrument kind, plus two entry-level
/// string fields. Matches a realistic mid-sized service event.
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

/// Discards everything written. Reports `Ok(len)` so callers see a "complete"
/// write. Lets the EMF bench measure format cost without I/O noise.
struct NullWriter;

impl io::Write for NullWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn make_otel_sink() -> OtelSink {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter).build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
    OtelSink::builder()
        .with_meter_provider(meter_provider)
        .build()
}

fn make_emf_sink() -> FlushImmediately<MixedEntry, impl metrique_writer::EntryIoStream> {
    FlushImmediately::new(Emf::all_validations("Bench".into(), vec![vec![]]).output_to(NullWriter))
}

/// One-shot pre-measurement of the EMF serialized size for one `MixedEntry`.
/// Counts bytes written to a `Vec<u8>` through the same format pipeline.
fn measure_emf_bytes() -> u64 {
    use std::sync::{Arc, Mutex};

    struct SharedCounter(Arc<Mutex<u64>>);
    impl io::Write for SharedCounter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            *self.0.lock().unwrap() += buf.len() as u64;
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let total = Arc::new(Mutex::new(0u64));
    let sink: FlushImmediately<MixedEntry, _> = FlushImmediately::new(
        Emf::all_validations("Bench".into(), vec![vec![]])
            .output_to(SharedCounter(Arc::clone(&total))),
    );
    sink.append(MixedEntry::fresh());
    *total.lock().unwrap()
}

#[divan::bench]
fn format_comparison_otel(bencher: Bencher) {
    let sink = make_otel_sink();
    // Warm the instrument cache so the first iteration's first-sight cost
    // doesn't skew the timing.
    sink.append(MixedEntry::fresh());
    bencher
        .counter(1usize)
        .with_inputs(MixedEntry::fresh)
        .bench_values(|entry| sink.append(black_box(entry)));
}

#[divan::bench]
fn format_comparison_emf(bencher: Bencher) {
    let bytes_per_entry = measure_emf_bytes();
    let sink = make_emf_sink();
    bencher
        .counter(BytesCount::new(bytes_per_entry))
        .with_inputs(MixedEntry::fresh)
        .bench_values(|entry| sink.append(black_box(entry)));
}
