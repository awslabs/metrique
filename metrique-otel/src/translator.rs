// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;

use metrique_writer_core::{
    EntryConfig, MetricFlags, Observation, Unit, ValidationError,
    entry::EntryWriter,
    value::{Distribution, Value, ValueWriter},
};
use opentelemetry::KeyValue;

use crate::{
    flags::{InstrumentKind, OtelOptions},
    metrics::InstrumentCache,
};

/// A pending metric observation captured during `Entry::write`, replayed
/// against the instrument cache once we have the full entry-level attribute
/// set. Buffering is what lets a string field declared *after* a metric
/// field still ride along as an attribute on that metric.
struct PendingMetric {
    name: String,
    kind: InstrumentKind,
    observations: Vec<Observation>,
    unit: Unit,
    per_metric_dimensions: Vec<KeyValue>,
}

pub(crate) struct OtelEntryWriter<'sink> {
    pub(crate) cache: &'sink InstrumentCache,
    /// String fields collected during the walk; applied as attributes to
    /// every metric in this entry at `finish()` time.
    entry_attributes: Vec<KeyValue>,
    pending: Vec<PendingMetric>,
}

impl<'sink> OtelEntryWriter<'sink> {
    pub(crate) fn new(cache: &'sink InstrumentCache) -> Self {
        Self {
            cache,
            entry_attributes: Vec::new(),
            pending: Vec::new(),
        }
    }

    pub(crate) fn finish(self) {
        for m in self.pending {
            // Per-metric dimensions take precedence by appearing first; the
            // entry-level attributes follow. The OTEL SDK does not de-dup
            // attribute keys, so any collision is left visible — that's a
            // user-data problem, not something to paper over here.
            let mut attributes = m.per_metric_dimensions;
            attributes.extend(self.entry_attributes.iter().cloned());
            self.cache
                .record(&m.name, m.kind, m.observations, m.unit, &attributes);
        }
    }
}

impl<'a, 'sink> EntryWriter<'a> for OtelEntryWriter<'sink> {
    fn timestamp(&mut self, _timestamp: std::time::SystemTime) {
        // OTEL meter readers stamp measurements with their own clock; the
        // entry timestamp is informational only. Until the descriptor system
        // (#282) gives us a structural way to surface it (e.g. as an
        // attribute or via a source extractor), it's dropped here.
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
        // String fields become entry-wide attributes attached to every
        // metric this entry produces. Keeping metadata next to metrics is
        // the explicit V1 goal — see plan items 1 and 2.
        self.parent
            .entry_attributes
            .push(KeyValue::new(self.name.into_owned(), value.to_owned()));
    }

    fn metric<'b>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'b str, &'b str)>,
        flags: MetricFlags<'_>,
    ) {
        // Resolve the instrument kind from the metric flags:
        //   - explicit `OtelOptions` (from a `Counter`/`Histogram`/etc. wrapper) wins
        //   - `Distribution` (set by `metrique-aggregation`'s closed histograms)
        //     maps to a histogram instrument
        //   - otherwise we drop the observation; picking a default would mask
        //     user bugs (forgetting to tag the instrument kind).
        let kind = if let Some(opts) = flags.downcast::<OtelOptions>() {
            opts.kind
        } else if flags.downcast::<Distribution>().is_some() {
            InstrumentKind::Histogram
        } else {
            return;
        };
        let per_metric_dimensions: Vec<KeyValue> = dimensions
            .into_iter()
            .map(|(k, v)| KeyValue::new(k.to_owned(), v.to_owned()))
            .collect();
        self.parent.pending.push(PendingMetric {
            name: self.name.into_owned(),
            kind,
            observations: distribution.into_iter().collect(),
            unit,
            per_metric_dimensions,
        });
    }

    fn error(self, _error: ValidationError) {
        // Validation errors are silently dropped for now.
    }
}
