// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A [Value] is something that can be written to a given metric field.
//!
//! This includes both numeric ([Observation], so scalar or distribution) values,
//! as well as string properties.

mod dimensions;
mod flags;
mod force;
mod formatter;
mod primitive;
mod quantized;

pub use dimensions::{WithDimension, WithDimensions, WithVecDimensions};
pub use force::{FlagConstructor, ForceFlag, ForceFlagEntryWriter};
pub use formatter::{FormattedValue, Lifted, NotLifted, ToString, ValueFormatter};
pub use quantized::{Quantized, QuantizingValueWriter, quantize_observation};
use std::{borrow::Cow, fmt::Write, sync::Arc};

pub use flags::{Distribution, MetricFlags, MetricOptions};

use crate::{
    CowStr, Unit, ValidationError,
    unit::UnitTag,
    unit::{self, Convert, WithUnit},
};

/// A metric value that may be associated with a name in a [`crate::EntryWriter::value()`] call.
///
/// A value can emit either nothing, a string, an [`Observation`] containing any number of scalars,
/// or a [`ValidationError`].
///
/// This differs from [`Entry`] because an [`Entry`] that emits a single value has to emit it to a
/// specific metric name, while a [`Value`] has the name passed from outside.
///
/// [`Entry`]: crate::Entry
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a metric value",
    note = "If `{Self}` is a metric *entry*, flatten it using `#[metrics(flatten)]`"
)]
pub trait Value {
    /// The statically-known shape of this value type for descriptor-aware sinks.
    ///
    /// Defaults to [`Opaque`](crate::descriptor::FieldShape::Opaque) when not overridden. Sinks use this to
    /// determine wire encoding without observing a live write.
    #[cfg(not(metrique_require_explicit_impls))]
    const SHAPE: crate::descriptor::FieldShape<'static> = crate::descriptor::FieldShape::Opaque;
    /// The statically-known shape of this value type for descriptor-aware sinks.
    #[cfg(metrique_require_explicit_impls)]
    const SHAPE: crate::descriptor::FieldShape<'static>;

    /// The unit this value type is reported in.
    ///
    /// Defaults to [`Unit::None`] (unitless). Types with inherent units (like
    /// `Duration`, which reports in milliseconds) override this. Explicit
    /// `#[metrics(unit = X)]` takes precedence.
    #[cfg(not(metrique_require_explicit_impls))]
    const UNIT: crate::Unit = crate::Unit::None;
    /// The unit this value type is reported in.
    #[cfg(metrique_require_explicit_impls)]
    const UNIT: crate::Unit;

    /// Write the value to the metric entry. This must never panic, but invalid values may trigger a validaiton panic on
    /// [`crate::EntrySink::append()`] for test sinks or a `tracing` event on production queues.
    fn write(&self, writer: impl ValueWriter);
}

/// Provided by a format for each call to [`crate::EntryWriter::value()`].
///
/// # Note for wrapper implementations
///
/// Implementations that wrap another `ValueWriter` (forwarding calls to it, possibly
/// modifying them) must also forward [`values()`](ValueWriter::values). Relying on the
/// default implementation silently downgrades lists to a comma-joined string, bypassing
/// the inner writer's native array support and dropping any per-element adjustments the
/// wrapper makes in `metric()`.
pub trait ValueWriter: Sized {
    /// Write an arbitrary string property to the entry. This may populate entry-wide dimensions in EMF.
    ///
    /// This must never panic, but if format-invalid characters are included it may trigger a panic on
    /// [`crate::EntrySink::append()`] for test sinks or a `tracing` event on production queues.
    fn string(self, value: &str);

    /// Write an arbitrary metric value to the entry. The value `distribution` can be a single numeric [`Observation`]
    /// or a sum of multiple observations. Some metric formats can preserve aspects of a multi-valued distribution,
    /// like the average and count, while others will only report the sum. Note that most formats do not support
    /// negative observations.
    ///
    /// It's possible for a metric to have no observations (the distribution is an empty iteration). These are
    /// normally ignored by the [format](crate::format::Format) if their other attributes are valid, but might
    /// still cause validation errors if invalid in other ways (e.g. duplicate).
    ///
    /// `dimensions` can be an arbitrary set of (dimension, instance) pairs attached to this individual value. Not all
    /// formats support per-value dimensions (e.g. EMF).
    ///
    /// This must never panic, but if unsupported values, units, or dimensions are included it may trigger a panic on
    /// [`crate::EntrySink::append()`] for test sinks or a `tracing` event on production queues.
    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        flags: MetricFlags<'_>,
    );

    /// Record an error rather than writing out a value.
    ///
    /// This should occur if the value can't be correctly written (e.g. a `NaN` floating point value).
    fn error(self, error: ValidationError);

    /// Shortcut to reporting an invalidation reason as a string.
    fn invalid(self, reason: impl Into<String>) {
        self.error(ValidationError::invalid(reason))
    }

    /// Write a list of values. Formats that support native arrays (e.g. EMF) can override this
    /// to emit a structured representation. The default is [`write_values_as_string`], which
    /// comma-joins each element's string representation.
    ///
    /// Wrapper writers must forward this to the writer they wrap (see the trait-level note).
    #[cfg(not(metrique_require_explicit_impls))]
    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        write_values_as_string(self, values)
    }

    /// Write a list of values. See the non-`metrique_require_explicit_impls` version, which
    /// documents this method and defaults it to [`write_values_as_string`].
    #[cfg(metrique_require_explicit_impls)]
    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>);
}

// Inline capacity for the buffer a wrapper `ValueWriter` needs when forwarding `values`:
// the inner writer takes an iterator of references, so re-wrapped elements have to be
// materialized first. Elements are 8-24 bytes, so this costs at most ~200 bytes of stack and
// spills to the heap beyond 8. 8 is what these buffers used before the const existed, not a
// measured optimum.
#[doc(hidden)]
pub const VALUES_INLINE_CAPACITY: usize = 8;

/// The fallback [`ValueWriter::values`] behaviour: comma-join each element's string representation
/// into a single [`ValueWriter::string`] call, skipping elements that write nothing (e.g. `None`).
/// An empty list still calls `string("")`.
///
/// This is lossy: per-element metric attributes (units, dimensions, flags) are dropped, and formats
/// with native array support never see the individual elements. Writers that wrap another
/// `ValueWriter` should forward `values` instead of calling this.
pub fn write_values_as_string<'a, V: Value + 'a>(
    writer: impl ValueWriter,
    values: impl IntoIterator<Item = &'a V>,
) {
    let mut buf = String::new();
    for value in values {
        let before = buf.len();
        if !buf.is_empty() {
            buf.push(',');
        }
        let after_sep = buf.len();
        value.write(StringCapture(&mut buf));
        if buf.len() <= after_sep {
            buf.truncate(before);
        }
    }
    writer.string(&buf);
}

/// Adapter that captures a [`Value`]'s string representation into a buffer.
/// Strings are appended directly. Metric observations are written as their
/// numeric string representation, comma-separated within a single element.
pub(crate) struct StringCapture<'a>(pub(crate) &'a mut String);

impl ValueWriter for StringCapture<'_> {
    fn string(self, value: &str) {
        self.0.push_str(value);
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        _unit: Unit,
        _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        let mut first = true;
        for obs in distribution {
            if !first {
                self.0.push(',');
            }
            first = false;
            match obs {
                Observation::Unsigned(v) => {
                    let _ = write!(self.0, "{v}");
                }
                Observation::Floating(v) => {
                    let _ = write!(self.0, "{v}");
                }
                Observation::Repeated { total, .. } => {
                    let _ = write!(self.0, "{total}");
                }
            }
        }
    }

    fn error(self, _error: ValidationError) {}

    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        // A list nested inside a list element flattens into the joined representation.
        write_values_as_string(self, values)
    }
}

/// The numeric value of a observation to include in a metric value.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Observation {
    /// A numeric observation
    Unsigned(u64),
    /// Note that most formats do not support `NaN`, negative, or infinite floating point values.
    Floating(f64),
    /// The result of summing `occurrences` into a single `total`. See [`ValueWriter::metric()`].
    ///
    /// It is fine for `occurrences` to be 0, and should not result in a validation
    /// error or a panic. If `occurrences` is 0 and `total` is not 0, the formatter behavior
    /// might not be what you expect (for example, the EMF formatter will ignore the
    /// `total`), but it should not cause an error or panic.
    Repeated {
        /// The total sum of occurrences
        total: f64,
        /// The number of occurrences
        occurrences: u64,
    },
}

impl Value for Observation {
    const SHAPE: crate::descriptor::FieldShape<'static> =
        crate::descriptor::FieldShape::Known(crate::descriptor::KnownShape::F64);
    const UNIT: crate::Unit = crate::Unit::None;

    fn write(&self, writer: impl ValueWriter) {
        writer.metric([*self], unit::None::UNIT, [], MetricFlags::empty())
    }
}

impl MetricValue for Observation {
    type Unit = unit::None;
}

/// A [`Value`] type that promises to write a metric with unit [`MetricValue::Unit`].
///
/// Implementations that invoke [`ValueWriter::metric`] with a different unit may trigger a [`ValidationError`].
pub trait MetricValue: Value {
    /// The [UnitTag] the metric will be emitted at
    type Unit: UnitTag;

    /// Convert this value to the given [`Unit`] when being written.
    fn with_unit<U: UnitTag>(self) -> WithUnit<Self, U>
    where
        Self: Sized,
        Self::Unit: Convert<U>,
    {
        self.into()
    }

    /// Add a dimension `(key, value)` when being written.
    ///
    /// This does *not* clear any existing dimensions.
    fn with_dimension(self, key: impl Into<CowStr>, value: impl Into<CowStr>) -> WithDimension<Self>
    where
        Self: Sized,
    {
        WithDimension::new(self, key, value)
    }

    /// Add a series of dimensions when being written.
    ///
    /// This does *not* clear any existing dimensions.
    fn with_dimensions<C, I, const N: usize>(
        self,
        dimensions: impl IntoIterator<Item = (C, I)>,
    ) -> WithDimensions<Self, N>
    where
        Self: Sized,
        C: Into<CowStr>,
        I: Into<CowStr>,
    {
        WithDimensions::new_with_dimensions(self, dimensions)
    }

    /// Reduce the precision of this value when it is written, retaining `bits` significant
    /// bits and using `rounding` to pick each bucket's representative.
    ///
    /// This trades a bounded amount of accuracy for a smaller set of distinct emitted values.
    /// See the [`quantize`](crate::quantize) module for the error bound at each bit count and
    /// for guidance on which values should not be quantized.
    ///
    /// If a unit conversion is also needed, convert first: the error bound applies to the
    /// value as emitted.
    ///
    /// ```
    /// use metrique_writer_core::quantize::{Rounding, SignificantBits};
    /// use metrique_writer_core::value::MetricValue;
    ///
    /// let bits = SignificantBits::new(8).unwrap();
    ///
    /// // At 8 bits the emitted value is within 0.390625% of the true one.
    /// let latency = 1_234_567u64.quantized(bits, Rounding::Midpoint);
    /// assert_eq!(*latency, 1_234_567);
    /// ```
    fn quantized(
        self,
        bits: crate::quantize::SignificantBits,
        rounding: crate::quantize::Rounding,
    ) -> Quantized<Self, crate::quantize::Quantizer>
    where
        Self: Sized,
    {
        Quantized::new(self, crate::quantize::Quantizer::new(bits, rounding))
    }
}

// Delegate Value impls for references and standard containers

impl<T: Value + ?Sized> Value for &T {
    const SHAPE: crate::descriptor::FieldShape<'static> = T::SHAPE;
    const UNIT: crate::Unit = T::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: Value> Value for Option<T> {
    const SHAPE: crate::descriptor::FieldShape<'static> =
        crate::descriptor::FieldShape::Optional(crate::descriptor::ShapeRef::new(&T::SHAPE));
    const UNIT: crate::Unit = T::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        if let Some(data) = self.as_ref() {
            data.write(writer)
        }
    }
}

impl<T: Value> Value for Box<T> {
    const SHAPE: crate::descriptor::FieldShape<'static> = T::SHAPE;
    const UNIT: crate::Unit = T::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: Value + ?Sized> Value for Arc<T> {
    const SHAPE: crate::descriptor::FieldShape<'static> = T::SHAPE;
    const UNIT: crate::Unit = T::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: Value + ToOwned + ?Sized> Value for Cow<'_, T> {
    const SHAPE: crate::descriptor::FieldShape<'static> = T::SHAPE;
    const UNIT: crate::Unit = T::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: MetricValue + ?Sized> MetricValue for &T {
    type Unit = T::Unit;
}

impl<T: MetricValue> MetricValue for Option<T> {
    type Unit = T::Unit;
}

impl<T: MetricValue> MetricValue for Box<T> {
    type Unit = T::Unit;
}

impl<T: MetricValue + ?Sized> MetricValue for Arc<T> {
    type Unit = T::Unit;
}

impl<T: MetricValue + ToOwned + ?Sized> MetricValue for Cow<'_, T> {
    type Unit = T::Unit;
}
