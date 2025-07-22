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

pub use dimensions::{WithDimension, WithDimensions, WithVecDimensions};
pub use force::{FlagConstructor, ForceFlag};
pub use formatter::{FormattedValue, ValueFormatter};
use std::{borrow::Cow, sync::Arc};

pub use flags::{MetricFlags, MetricOptions};

use crate::{
    CowStr, Unit, ValidationError,
    unit::UnitTag,
    unit::{self, Convert, WithUnit},
};

/// A metric value that may be associated with a name in a [`crate::EntryWriter::value()`] call.
pub trait Value {
    /// Write the value to the metric entry. This must never panic, but invalid values may trigger a validaiton panic on
    /// [`crate::EntrySink::append()`] for test sinks or a `tracing` event on production queues.
    fn write(&self, writer: impl ValueWriter);
}

/// Provided by a format for each call to [`crate::EntryWriter::value()`].
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
}

// Delegate Value impls for references and standard containers

impl<T: Value + ?Sized> Value for &T {
    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: Value> Value for Option<T> {
    fn write(&self, writer: impl ValueWriter) {
        if let Some(data) = self.as_ref() {
            data.write(writer)
        }
    }
}

impl<T: Value> Value for Box<T> {
    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: Value> Value for Arc<T> {
    fn write(&self, writer: impl ValueWriter) {
        (**self).write(writer)
    }
}

impl<T: Value + ToOwned + ?Sized> Value for Cow<'_, T> {
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

impl<T: MetricValue> MetricValue for Arc<T> {
    type Unit = T::Unit;
}

impl<T: MetricValue + ToOwned + ?Sized> MetricValue for Cow<'_, T> {
    type Unit = T::Unit;
}
