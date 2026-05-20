// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Same metric entries delivered to **both** `OtelSink` and a `metrique-writer-format-emf`
//! pipeline. Useful while migrating from EMF to OTel, you can run them in
//! parallel and diff the output before flipping over.
//!
//! ## Why this needs a small inline helper
//!
//! `metrique-writer` ships `stream::tee()` for fanning one entry to multiple
//! destinations, but it only works on `EntryIoStream` (format-based streams
//! like EMF). `OtelSink` is an `EntrySink`, not an `EntryIoStream`, so we
//! can't tee them at the stream layer. Instead, this example defines a
//! tiny `Fanout` `EntrySink` adapter that holds two child sinks and
//! appends each entry to both.
//!
//! Because `Fanout::append` takes the entry by value and forwards it twice,
//! it requires `E: Clone`. That rules out `BoxEntry` (not `Clone`), so we
//! work with a concrete, hand-rolled `Entry` type rather than going
//! through the global `ServiceMetrics` registration.
//!
//! ## Running this example
//!
//! ```sh
//! docker run --rm -p 4317:4317 -p 4318:4318 \
//!     otel/opentelemetry-collector-contrib:latest
//!
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
//! OTEL_SERVICE_NAME=metrique-otel-example \
//!     cargo run -p metrique-otel --example otlp_alongside_emf
//! ```
//!
//! You'll see EMF JSON lines on stdout AND the same metrics reaching the
//! OTLP collector.

use std::borrow::Cow;
use std::time::{Instant, SystemTime};

use metrique_otel::OtelSink;
use metrique_otel::flags::{Counter, Histogram};
use metrique_writer::sink::BackgroundQueue;
use metrique_writer::{EntrySink, FormatExt};
use metrique_writer_core::sink::FlushWait;
use metrique_writer_core::value::ForceFlag;
use metrique_writer_core::{Entry, EntryWriter};
use metrique_writer_format_emf::Emf;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

/// Concrete entry type. Built by hand (rather than via `#[metrics]`) so that
/// it can `derive(Clone)`. `Fanout` needs to send a copy of each entry to
/// each downstream sink.
#[derive(Clone)]
struct RequestEntry {
    operation: String,
    request_count: ForceFlag<u64, Counter>,
    latency_ms: ForceFlag<f64, Histogram>,
}

impl Entry for RequestEntry {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        w.timestamp(SystemTime::now());
        // String fields become per-entry attributes / EMF dimensions.
        w.value(Cow::Borrowed("Operation"), &self.operation);
        w.value(Cow::Borrowed("RequestCount"), &self.request_count);
        w.value(Cow::Borrowed("LatencyMs"), &self.latency_ms);
    }
}

/// Tiny inline tee for `EntrySink`. Forwards each appended entry to both
/// children. Production code would likely choose `tee()` at the
/// `EntryIoStream` level when possible. This combinator exists for the
/// EntrySink-only case (any `EntrySink` that isn't backed by an
/// `EntryIoStream`, e.g. `OtelSink`).
#[derive(Clone)]
struct Fanout<A, B> {
    a: A,
    b: B,
}

impl<E, A, B> EntrySink<E> for Fanout<A, B>
where
    E: Entry + Clone + Send + 'static,
    A: EntrySink<E> + Send + Sync,
    B: EntrySink<E> + Send + Sync,
{
    fn append(&self, entry: E) {
        self.a.append(entry.clone());
        self.b.append(entry);
    }

    fn flush_async(&self) -> FlushWait {
        let a = self.a.flush_async();
        let b = self.b.flush_async();
        FlushWait::from_future(async move {
            let _ = tokio::join!(a, b);
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // --- OTel pipeline ---
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("OTLP env not configured");
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter).build())
        .build();
    let otel_sink = OtelSink::builder()
        .with_meter_provider(meter_provider)
        .with_scope("metrique/otlp_alongside_emf")
        .build();

    // --- EMF pipeline ---
    // EMF is a format-based EntryIoStream; wrap it in BackgroundQueue to
    // get an EntrySink<RequestEntry> we can tee with. Keep `_emf_join`
    // bound until end of main, dropping it shuts the queue down (with up
    // to a 5-minute drain).
    let emf_stream = Emf::all_validations("MetriqueOtelExample".to_owned(), vec![vec![]])
        .output_to_makewriter(|| std::io::stdout().lock());
    let (emf_sink, _emf_join): (BackgroundQueue<RequestEntry>, _) =
        BackgroundQueue::new(emf_stream);

    // --- Tee ---
    let sink = Fanout {
        a: otel_sink.clone(),
        b: emf_sink,
    };

    for op in ["GET", "POST"] {
        let start = Instant::now();
        sink.append(RequestEntry {
            operation: op.to_owned(),
            request_count: ForceFlag::from(1u64),
            latency_ms: ForceFlag::from(start.elapsed().as_secs_f64() * 1000.0),
        });
    }

    // Drain both pipes. Fanout's flush_async awaits each child concurrently.
    sink.flush_async().await;
}
