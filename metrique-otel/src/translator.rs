// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::cell::RefCell;

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

impl PendingMetric {
    /// Reset fields to a clean state while keeping the heap capacities of
    /// `name`, `observations`, and `per_metric_dimensions`. Used when a
    /// slot is recycled out of [`WriterBufs::metric_pool`]; `kind` and
    /// `unit` are placeholders, overwritten before the slot leaves the
    /// pool.
    fn empty() -> Self {
        Self {
            name: String::new(),
            kind: InstrumentKind::Counter,
            observations: Vec::new(),
            unit: Unit::None,
            per_metric_dimensions: Vec::new(),
        }
    }
}

/// Per-thread scratch space reused across appends. Outer `Vec`s and the
/// inner `Vec`s inside each pooled `PendingMetric` keep their capacity
/// across calls, so steady-state appends on a given thread allocate only
/// for genuine growth (e.g. a wider entry than seen before).
pub(crate) struct WriterBufs {
    entry_attributes: Vec<KeyValue>,
    pending: Vec<PendingMetric>,
    /// Scratch used in `finish` to merge per-metric dimensions with the
    /// entry-level attributes; only touched when at least one metric has
    /// per-metric dimensions.
    scratch: Vec<KeyValue>,
    /// Recycled `PendingMetric` slots with their inner `Vec` capacity
    /// preserved.
    metric_pool: Vec<PendingMetric>,
}

impl WriterBufs {
    pub(crate) const fn new() -> Self {
        Self {
            entry_attributes: Vec::new(),
            pending: Vec::new(),
            scratch: Vec::new(),
            metric_pool: Vec::new(),
        }
    }

    /// Reset the per-append state. Tolerates leftover state from a prior
    /// `Entry::write` that panicked mid-walk: any half-built `pending`
    /// slots get returned to the pool with their capacity intact.
    fn reset(&mut self) {
        self.entry_attributes.clear();
        self.scratch.clear();
        while let Some(mut m) = self.pending.pop() {
            m.name.clear();
            m.observations.clear();
            m.per_metric_dimensions.clear();
            self.metric_pool.push(m);
        }
    }

    fn take_metric(&mut self) -> PendingMetric {
        self.metric_pool.pop().unwrap_or_else(PendingMetric::empty)
    }
}

thread_local! {
    static BUFS: RefCell<WriterBufs> = const { RefCell::new(WriterBufs::new()) };
}

/// Run a single append against the per-thread `WriterBufs` arena. Falls
/// back to a stack-local arena if the thread-local is unavailable
/// (re-entrant append from inside `Entry::write`, or thread shutdown).
pub(crate) fn append_with_pool(
    cache: &InstrumentCache,
    scope: &'static str,
    entry: &(impl metrique_writer_core::Entry + ?Sized),
) {
    let mut ran = false;
    let _ = BUFS.try_with(|cell| {
        if let Ok(mut bufs) = cell.try_borrow_mut() {
            run(cache, scope, &mut bufs, entry);
            ran = true;
        }
    });
    if !ran {
        let mut local = WriterBufs::new();
        run(cache, scope, &mut local, entry);
    }
}

fn run(
    cache: &InstrumentCache,
    scope: &'static str,
    bufs: &mut WriterBufs,
    entry: &(impl metrique_writer_core::Entry + ?Sized),
) {
    bufs.reset();
    let mut writer = OtelEntryWriter { cache, scope, bufs };
    entry.write(&mut writer);
    writer.finish();
}

pub(crate) struct OtelEntryWriter<'a> {
    pub(crate) cache: &'a InstrumentCache,
    /// OTel `InstrumentationScope` name, derived once per entry type at
    /// [`OtelSink::append`] time from `std::any::type_name::<E>()`.
    ///
    /// [`OtelSink::append`]: crate::OtelSink
    pub(crate) scope: &'static str,
    bufs: &'a mut WriterBufs,
}

impl<'a> OtelEntryWriter<'a> {
    fn finish(self) {
        let OtelEntryWriter { cache, scope, bufs } = self;
        // Pop metrics in reverse insertion order; OTel doesn't care about
        // record ordering inside an export window, and popping lets us
        // return each slot to the pool without holding a second borrow.
        while let Some(mut m) = bufs.pending.pop() {
            let attrs: &[KeyValue] = if m.per_metric_dimensions.is_empty() {
                // Common case: no per-metric dimensions on this metric.
                // Borrow the entry-level attribute set directly, skipping
                // the N*K clone the old code paid.
                &bufs.entry_attributes
            } else {
                // Uncommon path: combine per-metric dims (first, so they
                // take precedence on the wire) with the entry-level
                // attrs. Reuses a single scratch Vec across metrics. The
                // OTEL SDK does not de-dup attribute keys, so any
                // collision is left visible — that's a user-data problem,
                // not something to paper over here.
                bufs.scratch.clear();
                bufs.scratch.append(&mut m.per_metric_dimensions);
                bufs.scratch.extend(bufs.entry_attributes.iter().cloned());
                &bufs.scratch
            };
            cache.record(
                scope,
                &m.name,
                m.kind,
                m.observations.drain(..),
                m.unit,
                attrs,
            );
            m.name.clear();
            bufs.metric_pool.push(m);
        }
    }
}

impl<'a, 'b> EntryWriter<'a> for OtelEntryWriter<'b> {
    /// Dropped. OTel meter readers stamp measurements with their own clock,
    /// so the entry timestamp is informational only on this path. Until the
    /// descriptor system (#282) gives us a structural way to surface it
    /// (e.g. as an attribute or via a source extractor), it's discarded.
    fn timestamp(&mut self, _timestamp: std::time::SystemTime) {}

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        let writer = OtelValueWriter { parent: self, name };
        value.write(writer);
    }

    fn config(&mut self, _config: &'a dyn EntryConfig) {
        // OTEL-specific entry config is not consumed yet.
    }
}

pub(crate) struct OtelValueWriter<'a, 'b> {
    pub(crate) parent: &'a mut OtelEntryWriter<'b>,
    pub(crate) name: Cow<'a, str>,
}

impl<'a, 'b> ValueWriter for OtelValueWriter<'a, 'b> {
    fn string(self, value: &str) {
        // String fields become entry-wide attributes attached to every
        // metric this entry produces. Keeping metadata next to metrics is
        // the explicit V1 goal — see plan items 1 and 2.
        self.parent
            .bufs
            .entry_attributes
            .push(KeyValue::new(self.name.into_owned(), value.to_owned()));
    }

    fn metric<'c>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'c str, &'c str)>,
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
        let mut metric = self.parent.bufs.take_metric();
        metric.name.clear();
        metric.name.push_str(&self.name);
        metric.kind = kind;
        metric.unit = unit;
        metric.observations.clear();
        metric.observations.extend(distribution);
        metric.per_metric_dimensions.clear();
        metric.per_metric_dimensions.extend(
            dimensions
                .into_iter()
                .map(|(k, v)| KeyValue::new(k.to_owned(), v.to_owned())),
        );
        self.parent.bufs.pending.push(metric);
    }

    fn error(self, _error: ValidationError) {
        // Validation errors are silently dropped for now.
    }
}
