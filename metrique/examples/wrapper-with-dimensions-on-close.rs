// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This example demonstrates how to create a wrapper that automatically wraps
//! each field in `WithDimensions` when the metrics are closed.
//!
//! The key insight is that a field on its own doesn't become a dimension — you
//! need to use the `WithDimensions` wrapper. While `WithDimensions` normally only
//! works for individual fields, we can create a wrapper struct that automatically
//! applies `WithDimensions` to ALL fields by implementing a custom `EntryWriter`
//! that wraps each value as it's written.

use std::{borrow::Cow, time::SystemTime};

use metrique::{
    CloseValue, Counter, InflectableEntry, append_and_close, timers::Timer, unit_of_work::metrics,
};
use metrique_writer::{
    AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink, sink::global_entry_sink,
};
use metrique_writer_core::{
    EntryConfig, EntryWriter, MetricFlags, Observation, Unit, Value, ValueWriter,
    config::AllowSplitEntries, entry::SampleGroupElement,
};
use metrique_writer_format_emf::Emf;

global_entry_sink! { ServiceMetrics }

// Your original metrics struct
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    request_id: String,

    #[metrics(timestamp)]
    timestamp: SystemTime,

    request_count: Counter,
    processing_time: Timer,
    error_count: Counter,
}

// Wrapper that automatically applies WithDimensions to all fields
struct WithDimensionsOnAllFields<E> {
    entry: E,
    dimensions: Vec<(CowStr, CowStr)>,
}

impl<E> WithDimensionsOnAllFields<E> {
    fn new(entry: E, dimensions: Vec<(String, String)>) -> Self {
        Self { entry, dimensions: dimensions.into_iter().map(|(k, v)|(k.into(), v.into())).collect() }
    }

    fn with_service_context(
        entry: E,
        service: String,
        version: String,
        environment: String,
    ) -> Self {
        Self::new(
            entry,
            vec![
                ("Service".to_string(), service),
                ("Version".to_string(), version),
                ("Environment".to_string(), environment),
            ],
        )
    }
}

// Implement CloseValue so this can be used with append_and_close
impl<E: CloseValue> CloseValue for WithDimensionsOnAllFields<E> {
    type Closed = WithDimensionsOnAllFieldsClosed<E::Closed>;

    fn close(self) -> Self::Closed {
        WithDimensionsOnAllFieldsClosed {
            entry: self.entry.close(),
            dimensions: self.dimensions,
        }
    }
}

// The closed version that implements Entry
struct WithDimensionsOnAllFieldsClosed<E> {
    entry: E,
    dimensions: Vec<(CowStr, CowStr)>,
}

// Implement InflectableEntry so this can be used with the metrics system
impl<E: InflectableEntry> InflectableEntry for WithDimensionsOnAllFieldsClosed<E> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        // Create a wrapper around the EntryWriter that automatically applies WithDimensions
        let mut wrapper = DimensionWrappingEntryWriter {
            writer,
            dimensions: &self.dimensions,
        };

        // Write the inner entry through our wrapper
        self.entry.write(&mut wrapper);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        self.entry.sample_group()
    }
}

// Custom EntryWriter that wraps each value in WithDimensions
struct DimensionWrappingEntryWriter<'a, W> {
    writer: W,
    dimensions: &'a [(CowStr, CowStr)],
}

impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for DimensionWrappingEntryWriter<'_, W> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.writer.timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        // Enable split entries so WithDimensions works properly
        self.writer.config(const { &AllowSplitEntries::new() });

        // This is the key: wrap each value in WithDimensions before writing
        let wrapped_value = ValueWithDimensions {
            value,
            dimensions: &self.dimensions,
        };

        self.writer.value(name, &wrapped_value);
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        self.writer.config(config);
    }
}

// Helper struct that wraps a value with dimensions
struct ValueWithDimensions<'a, V> {
    value: V,
    dimensions: &'a [(CowStr, CowStr)],
}

type CowStr = Cow<'static, str>;

impl<'a, V: Value> Value for ValueWithDimensions<'a, V> {
    fn write(&self, writer: impl ValueWriter) {
        // We'll use a custom ValueWriter that applies the dimensions
        struct Wrapper<'a, W> {
            writer: W,
            dimensions: &'a [(CowStr, CowStr)],
        }

        impl<W: ValueWriter> ValueWriter for Wrapper<'_, W> {
            fn string(self, value: &str) {
                self.writer.string(value);
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                flags: MetricFlags<'_>,
            ) {
                #[allow(clippy::map_identity)]
                // https://github.com/rust-lang/rust-clippy/issues/9280
                self.writer.metric(
                    distribution,
                    unit,
                    dimensions
                        .into_iter()
                        .map(|(k, v)| (k, v)) // reborrow to align lifetimes
                        .chain(self.dimensions.iter().map(|(c, i)| (&**c, &**i))),
                    flags,
                )
            }

            fn error(self, error: metrique_writer_core::ValidationError) {
                self.writer.error(error)
            }
        }

        let dimension_writer = Wrapper {
            writer,
            dimensions: self.dimensions
        };

        self.value.write(dimension_writer)
    }
}

// Extension trait for ergonomic usage
trait WithDimensionsOnAllFieldsExt: Sized {
    fn with_service_context(
        self,
        service: String,
        version: String,
        environment: String,
    ) -> WithDimensionsOnAllFields<Self> {
        WithDimensionsOnAllFields::with_service_context(self, service, version, environment)
    }
}

impl<T> WithDimensionsOnAllFieldsExt for T {}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("DimensionExample".to_string(), vec![vec![]])
            .output_to(std::io::stdout()),
    );

    println!("=== Request Metrics with Dimensions on All Fields ===");
    {
        let base_metrics = RequestMetrics {
            request_id: "req-123".to_string(),
            timestamp: SystemTime::now(),
            request_count: Default::default(),
            processing_time: Timer::start_now(),
            error_count: Default::default(),
        };

        // Create the wrapper that will apply WithDimensions to each field
        let wrapped_entry = base_metrics.with_service_context(
            "UserService".to_string(),
            "2.1.0".to_string(),
            "production".to_string(),
        );

        let mut metrics = append_and_close(wrapped_entry, ServiceMetrics::sink());

        // Modify the metrics
        metrics.entry.request_count.increment();

        // Simulate some work
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        metrics.entry.processing_time.stop();
        metrics.entry.error_count.set(0);

        // When metrics is dropped, each field (request_count, processing_time, error_count)
        // will be automatically wrapped in WithDimensions with the service context.
        // You can see in the output that all metrics share the same dimensions:
        // ["Environment","Service","Version"] with values "production", "UserService", "2.1.0"
    }

    // Give time for all metrics to flush
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}
