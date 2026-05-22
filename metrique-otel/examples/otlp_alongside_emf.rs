// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Same recording site delivering metrics to **both** an `OtelSink` (via
//! [`KeyedAggregator`]) and a `metrique-writer-format-emf` pipeline. Useful
//! during an EMF -> OTel migration, you can run them in parallel and diff the
//! output before flipping over.
//!
//! [`KeyedAggregator`]: metrique_aggregation::aggregator::KeyedAggregator
//!
//! ## Why two structs
//!
//! The OTel path here is aggregated: a `#[aggregate]` struct whose fields are
//! marked with strategies (`OtelCounter`, `OtelHistogram<U>`, ...). The
//! aggregator merges entries on a worker thread and only flushes one
//! observation per `#[aggregate(key)]` tuple per interval.
//!
//! The EMF path wants raw per-request entries (one event per request, with
//! string fields as dimensions). That is a different shape from the
//! aggregation source struct, so we declare two: `EmfRequest` for the EMF
//! event and `OtelRequest` for the aggregated OTel observation. The recording
//! site populates both from the same inputs.
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
//! OTLP collector (aggregated per `Operation`).

use std::borrow::Cow;
use std::time::{Duration, Instant, SystemTime};

use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_otel::OtelSink;
use metrique_otel::aggregate::{OtelCounter, OtelHistogram};
use metrique_writer::sink::BackgroundQueue;
use metrique_writer::{EntrySink, FormatExt};
use metrique_writer_core::{Entry, EntryWriter};
use metrique_writer_format_emf::Emf;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

/// EMF event: one entry per request. String fields become EMF dimensions,
/// numeric fields become EMF metric values. Hand-rolled `Entry` so the type
/// is directly usable with `BackgroundQueue<EmfRequest>`; the `#[metrics]`
/// derive produces a guard-based API that is a different idiom from the
/// direct-append flow we want here.
struct EmfRequest {
    operation: String,
    request_count: u64,
    latency_ms: f64,
}

impl Entry for EmfRequest {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        w.timestamp(SystemTime::now());
        w.value(Cow::Borrowed("Operation"), &self.operation);
        w.value(Cow::Borrowed("RequestCount"), &self.request_count);
        w.value(Cow::Borrowed("LatencyMs"), &self.latency_ms);
    }
}

/// OTel aggregated entry: rolls up by `Operation` on the worker thread; the
/// OTel SDK sees one observation per operation per flush.
#[aggregate]
#[metrics(rename_all = "PascalCase")]
struct OtelRequest {
    #[aggregate(key)]
    operation: String,
    #[aggregate(strategy = OtelCounter)]
    request_count: u64,
    #[aggregate(strategy = OtelHistogram<Millisecond>)]
    latency: Duration,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // --- OTel pipeline (aggregated) ---
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
    let otel_worker = WorkerSink::new(
        KeyedAggregator::<OtelRequest, _>::new(otel_sink.clone()),
        Duration::from_secs(1),
    );

    // --- EMF pipeline (direct) ---
    // EMF is a format-based EntryIoStream; wrap it in BackgroundQueue to get
    // an `EntrySink<EmfRequest>` we can append into. Keep `_emf_join` bound
    // until end of main; dropping it shuts the queue down (with up to a
    // 5-minute drain).
    let emf_stream = Emf::all_validations("MetriqueOtelExample".to_owned(), vec![vec![]])
        .output_to_makewriter(|| std::io::stdout().lock());
    let (emf_sink, _emf_join): (BackgroundQueue<EmfRequest>, _) = BackgroundQueue::new(emf_stream);

    // --- Recording site: populate both paths from the same inputs. ---
    for op in ["GET", "POST"] {
        let start = Instant::now();
        let latency = start.elapsed();
        let latency_ms = latency.as_secs_f64() * 1000.0;

        emf_sink.append(EmfRequest {
            operation: op.to_owned(),
            request_count: 1,
            latency_ms,
        });

        OtelRequest {
            operation: op.to_owned(),
            request_count: 1,
            latency,
        }
        .close_and_merge(otel_worker.clone());
    }

    // Drain both pipes.
    otel_worker.flush().await;
    otel_sink.flush_async().await;
    emf_sink.flush_async().await;
}
