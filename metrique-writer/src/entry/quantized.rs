// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Applying a quantizer to a whole stream of entries.

use std::{
    borrow::Cow,
    collections::HashSet,
    fmt,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::SystemTime,
};

use metrique_writer_core::{
    Entry, EntryConfig, EntryWriter, Value, quantize::Quantizer, value::Quantized,
};

use crate::CowStr;

/// Chooses which metrics a [`QuantizationPolicy`] applies to.
///
/// Kept private so that new selection strategies can be added without a breaking change; build
/// one with [`QuantizationPolicy::for_metrics`] or [`QuantizationPolicy::matching`].
#[derive(Clone)]
enum MetricFilter {
    // Both variants are behind an `Arc` so that cloning a `QuantizationPolicy` is a refcount
    // bump rather than a rehash of every name. Policies are cheap to share across streams.
    Names(Arc<HashSet<CowStr>>),
    Predicate(Arc<dyn Fn(&str) -> bool + Send + Sync>),
}

impl fmt::Debug for MetricFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricFilter::Names(names) => f.debug_tuple("Names").field(names).finish(),
            MetricFilter::Predicate(_) => f.write_str("Predicate(..)"),
        }
    }
}

/// Which metrics to quantize on a stream, and how.
///
/// A policy pairs a [`Quantizer`] with a filter naming the metrics it applies to. There is
/// deliberately **no** "quantize everything" constructor: a stream carries counts that must
/// reconcile exactly, identifiers that happen to be numeric, and other values whose precision
/// is load-bearing. Reducing those silently would be a bug, so the set of metrics to quantize
/// has to be stated.
///
/// Timestamps need no filtering. An entry's timestamp travels through
/// [`EntryWriter::timestamp`], never through [`EntryWriter::value`], so a decorator that only
/// intercepts values cannot reach it.
///
/// # Examples
///
/// Quantize two named latency metrics and nothing else:
///
/// ```
/// use metrique_writer::quantize::{Quantizer, Rounding, SignificantBits};
/// use metrique_writer::stream::QuantizationPolicy;
///
/// let quantizer = Quantizer::new(SignificantBits::new(8).unwrap(), Rounding::Midpoint);
/// let policy = QuantizationPolicy::for_metrics(quantizer, ["LatencyNanos", "DownstreamNanos"]);
///
/// assert!(policy.applies_to("LatencyNanos"));
/// assert!(!policy.applies_to("RequestCount"));
/// ```
///
/// Or select by a predicate, when the metric names follow a convention:
///
/// ```
/// use metrique_writer::quantize::Quantizer;
/// use metrique_writer::stream::QuantizationPolicy;
///
/// let policy = QuantizationPolicy::matching(Quantizer::default(), |name| {
///     name.ends_with("Nanos") || name.ends_with("Bytes")
/// });
///
/// assert!(policy.applies_to("LatencyNanos"));
/// assert!(policy.applies_to("PayloadBytes"));
/// assert!(!policy.applies_to("RequestCount"));
/// ```
#[derive(Clone, Debug)]
pub struct QuantizationPolicy {
    quantizer: Quantizer,
    filter: MetricFilter,
}

impl QuantizationPolicy {
    /// Quantize exactly the named metrics, matched by their emitted name.
    ///
    /// Names are compared after any renaming the entry applies, so they should match what
    /// appears in the output.
    pub fn for_metrics(
        quantizer: Quantizer,
        names: impl IntoIterator<Item = impl Into<CowStr>>,
    ) -> Self {
        Self {
            quantizer,
            filter: MetricFilter::Names(Arc::new(names.into_iter().map(Into::into).collect())),
        }
    }

    /// Quantize the metrics whose emitted name satisfies `predicate`.
    ///
    /// The predicate is consulted once per value written, so it should be cheap.
    pub fn matching(
        quantizer: Quantizer,
        predicate: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            quantizer,
            filter: MetricFilter::Predicate(Arc::new(predicate)),
        }
    }

    /// The quantizer this policy applies.
    pub fn quantizer(&self) -> Quantizer {
        self.quantizer
    }

    /// Whether the metric named `name` will be quantized.
    pub fn applies_to(&self, name: &str) -> bool {
        match &self.filter {
            MetricFilter::Names(names) => names.contains(name),
            MetricFilter::Predicate(predicate) => predicate(name),
        }
    }
}

/// An [`Entry`] whose matching values are reduced in precision as they are written.
///
/// Produced by [`EntryIoStreamExt::with_quantization`] and
/// [`FormatExt::with_quantization`]; see [`QuantizationPolicy`].
///
/// [`EntryIoStreamExt::with_quantization`]: crate::stream::EntryIoStreamExt::with_quantization
/// [`FormatExt::with_quantization`]: crate::format::FormatExt::with_quantization
/// The policy is borrowed rather than owned so that wrapping an entry costs nothing. A stream
/// builds one of these per entry, and cloning a policy per entry would put a refcount bump (or,
/// before the filter was `Arc`-backed, a full rehash of the name set) on the hot path.
#[derive(Clone, Copy, Debug)]
pub struct QuantizedEntry<'a, E> {
    entry: E,
    policy: &'a QuantizationPolicy,
}

impl<'a, E> QuantizedEntry<'a, E> {
    /// Reduce the precision of `entry`'s matching values when it is written.
    pub fn new(entry: E, policy: &'a QuantizationPolicy) -> Self {
        Self { entry, policy }
    }

    /// The policy being applied.
    pub fn policy(&self) -> &'a QuantizationPolicy {
        self.policy
    }
}

impl<E> Deref for QuantizedEntry<'_, E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl<E> DerefMut for QuantizedEntry<'_, E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entry
    }
}

impl<E: Entry> Entry for QuantizedEntry<'_, E> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        struct Wrapper<'a, W> {
            writer: W,
            policy: &'a QuantizationPolicy,
        }

        impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for Wrapper<'_, W> {
            fn timestamp(&mut self, timestamp: SystemTime) {
                // Forwarded untouched. Quantizing an epoch value would move it by hours, and
                // there is no path by which this decorator could do so: timestamps do not go
                // through `value()`.
                self.writer.timestamp(timestamp);
            }

            fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
                let name: Cow<'a, str> = name.into();
                if self.policy.applies_to(&name) {
                    self.writer
                        .value(name, &Quantized::new(value, self.policy.quantizer()));
                } else {
                    self.writer.value(name, value);
                }
            }

            fn config(&mut self, config: &'a dyn EntryConfig) {
                self.writer.config(config);
            }
        }

        self.entry.write(&mut Wrapper {
            writer,
            policy: self.policy,
        })
    }

    fn sample_group(
        &self,
    ) -> impl Iterator<Item = metrique_writer_core::entry::SampleGroupElement> {
        self.entry.sample_group()
    }

    fn descriptors(&self) -> metrique_writer_core::Descriptors<'_> {
        self.entry.descriptors()
    }
}
