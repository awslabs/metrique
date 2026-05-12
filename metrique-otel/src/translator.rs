// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{borrow::Cow, time::SystemTime};

use metrique_writer_core::{
    EntryConfig, MetricFlags, Observation, Unit, ValidationError,
    entry::EntryWriter,
    value::{Value, ValueWriter},
};
use opentelemetry::KeyValue;
use opentelemetry_sdk::logs::SdkLoggerProvider;

use crate::{flags::OtelOptions, logs::LogBuilder, metrics::InstrumentCache};

pub(crate) struct OtelEntryWriter<'sink> {
    pub(crate) cache: &'sink InstrumentCache,
    pub(crate) logger_provider: &'sink SdkLoggerProvider,
    pub(crate) log: LogBuilder,
}

impl<'sink> OtelEntryWriter<'sink> {
    pub(crate) fn new(
        cache: &'sink InstrumentCache,
        logger_provider: &'sink SdkLoggerProvider,
    ) -> Self {
        Self {
            cache,
            logger_provider,
            log: LogBuilder::new(),
        }
    }

    pub(crate) fn finish(self) {
        // Metrics are recorded eagerly during `value()`; the only thing left
        // for finalization is flushing the accumulated log record (if any
        // string field or timestamp was set).
        self.log.emit(self.logger_provider);
    }
}

impl<'a, 'sink> EntryWriter<'a> for OtelEntryWriter<'sink> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.log.set_timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        let writer = OtelValueWriter { parent: self, name };
        value.write(writer);
    }

    fn config(&mut self, _config: &'a dyn EntryConfig) {
        // OTEL-specific entry config is not consumed yet.
    }
}

pub(crate) struct OtelValueWriter<'a, 'sink> {
    pub(crate) parent: &'a mut OtelEntryWriter<'sink>,
    pub(crate) name: Cow<'a, str>,
}

impl<'a, 'sink> ValueWriter for OtelValueWriter<'a, 'sink> {
    fn string(self, value: &str) {
        self.parent.log.add_attribute(&self.name, value);
    }

    fn metric<'b>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'b str, &'b str)>,
        flags: MetricFlags<'_>,
    ) {
        // Without an OTEL flag we don't know the instrument kind, so we skip
        // the observation entirely. Picking a default would mask user bugs
        // (forgetting to wrap a field in `Counter`/`Histogram`/etc.).
        let Some(opts) = flags.downcast::<OtelOptions>() else {
            return;
        };
        let attributes: Vec<KeyValue> = dimensions
            .into_iter()
            .map(|(k, v)| KeyValue::new(k.to_owned(), v.to_owned()))
            .collect();
        self.parent
            .cache
            .record(&self.name, opts.kind, distribution, unit, &attributes);
    }

    fn error(self, _error: ValidationError) {
        // Validation errors are silently dropped for now; logging path will
        // surface them once Stage 5 lands.
    }
}
