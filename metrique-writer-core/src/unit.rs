// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains utilities for attaching [Unit]s (such as percents, kilobytes,
//! or seconds) to metrics. Conversion between different units is
//! handled by [Convert].
//!
//! Most metric systems have some way of attaching units to the uploaded
//! metrics, to make it obvious in which units of measue the uploaded
//! metrics are stored in.
//!
//! # Usage
//!
//! This is normally used via the [`WithUnit`] [Value]-wrapper.  For readability, prefer the
//! `As{Unit}` type aliases, like [`AsSeconds<T>`](`AsSeconds`) rather than
//! `WithUnit<T, Second>`.
//!
//! ```
//! # use metrique_writer::unit::{AsSeconds, AsBytes};
//! # use metrique_writer::Entry;
//! # use std::time::Duration;
//!
//! #[derive(Entry)]
//! struct MyEntry {
//!     my_timer: AsSeconds<Duration>,
//!     request_size: AsBytes<u64>,
//! }
//!
//! // `WithUnit` (and the aliases) implement `From`, initialize them like this:
//! MyEntry {
//!     my_timer: Duration::from_secs(2).into(),
//!     request_size: 2u64.into(),
//! };
//! ```

use std::{
    cmp::Ordering,
    fmt::{self, Debug, Display},
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{MetricValue, Observation, ValidationError, Value, ValueWriter, value::MetricFlags};

/// Represent all metric value units allowed by
/// [CloudWatch](https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_MetricDatum.html).
///
/// [`Unit::Custom`] provides an escape hatch for any unmodeled units.
#[non_exhaustive]
#[derive(Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    /// No Unit
    #[default]
    None,
    /// Count
    Count,
    /// Percent
    Percent,
    /// Seconds with a scale prefix
    Second(NegativeScale),
    /// Bytes with a scale prefix
    Byte(PositiveScale),
    /// Bytes/second with a scale prefix
    BytePerSecond(PositiveScale),
    /// Bits with a scale prefix
    Bit(PositiveScale),
    /// Bits/second with a scale prefix
    BitPerSecond(PositiveScale),
    /// Custom unit
    ///
    /// This is an escape hatch for units your format supports that
    /// are not in this enum
    ///
    /// Formatters will generally send the unit string
    /// directly to the metric format, so make sure the
    /// unit you put here is supported by your metric format.
    Custom(&'static str),
}

#[cfg(feature = "serde")]
impl serde::Serialize for Unit {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde::Serialize::serialize(self.name(), serializer)
    }
}

impl Unit {
    /// The public name defined by CloudWatch for the unit.
    pub const fn name(self) -> &'static str {
        macro_rules! positive_scale {
            ($scale:expr, $base:literal, $scaled:literal) => {
                match $scale {
                    PositiveScale::One => $base,
                    PositiveScale::Kilo => concat!("Kilo", $scaled),
                    PositiveScale::Mega => concat!("Mega", $scaled),
                    PositiveScale::Giga => concat!("Giga", $scaled),
                    PositiveScale::Tera => concat!("Tera", $scaled),
                }
            };
        }

        match self {
            Self::None => "None",
            Self::Count => "Count",
            Self::Percent => "Percent",
            Self::Second(scale) => match scale {
                NegativeScale::Micro => "Microseconds",
                NegativeScale::Milli => "Milliseconds",
                NegativeScale::One => "Seconds",
            },
            Self::Byte(scale) => positive_scale!(scale, "Bytes", "bytes"),
            Self::BytePerSecond(scale) => positive_scale!(scale, "Bytes/Second", "bytes/Second"),
            Self::Bit(scale) => positive_scale!(scale, "Bits", "bits"),
            Self::BitPerSecond(scale) => positive_scale!(scale, "Bits/Second", "bits/Second"),
            Self::Custom(unit) => unit,
        }
    }
}

impl fmt::Debug for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Supported *negative* power-of-ten scales for [`Unit`]s.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NegativeScale {
    /// `10^-6`
    Micro,
    /// `10^-3`
    Milli,
    #[default]
    /// `10^0`
    One,
}

/// Supported *positive* power-of-ten scales for [`Unit`]s.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PositiveScale {
    /// `10^0`
    #[default]
    One,
    /// `10^3`
    Kilo,
    /// `10^6`
    Mega,
    /// `10^9`
    Giga,
    /// `10^12`
    Tera,
}

impl NegativeScale {
    /// To convert from a [`Unit`] measured on this scale to the base unit, divide by this factor.
    ///
    /// ```
    /// # use metrique_writer_core::unit::NegativeScale;
    /// let milliseconds = 2000u64;
    /// let seconds = milliseconds/NegativeScale::Milli.reduction_factor();
    /// assert_eq!(seconds, 2);
    /// ```
    pub const fn reduction_factor(self) -> u64 {
        match self {
            Self::Micro => 1_000_000,
            Self::Milli => 1_000,
            Self::One => 1,
        }
    }
}

impl PositiveScale {
    /// To convert from a [`Unit`] measured on this scale to the base unit, multiply by this factor.
    ///
    /// ```
    /// # use metrique_writer_core::unit::PositiveScale;
    /// let megabytes = 42u64;
    /// let bytes = megabytes*PositiveScale::Mega.expansion_factor();
    /// assert_eq!(bytes, 42_000_000);
    /// ```
    pub const fn expansion_factor(self) -> u64 {
        match self {
            Self::One => 1,
            Self::Kilo => 1_000,
            Self::Mega => 1_000_000,
            Self::Giga => 1_000_000_000,
            Self::Tera => 1_000_000_000_000,
        }
    }
}

/// A marker trait that can be used to tag a value with a unit at compile time.
///
/// See [`crate::MetricValue`].
pub trait UnitTag {
    /// The [Unit] in the [UnitTag]
    const UNIT: Unit;
}

/// When implemented, signifies that values with the unit `Self` can be converted to the unit `U` by multiplying by
/// [`Convert::RATIO`].
///
/// For example, to convert from milliseconds to seconds:
/// ```
/// # use metrique_writer_core::{Convert, Observation, unit::{Millisecond, Second}};
/// let milliseconds = Observation::Floating(42.0);
/// let seconds = <Millisecond as Convert<Second>>::convert(milliseconds);
/// assert_eq!(seconds, Observation::Floating(0.042));
/// ```
///
/// Not all units can be freely converted (e.g. [`Second`]s can't be converted to [`Megabyte`]s).
///
/// ```compile_fail
/// # use metrique_writer_core::{Convert, Observation, unit::{Second, Megabyte}};
/// let seconds = Observation::Floating(42.0);
/// let mbs = <Second as Convert<Megabyte>>::convert(seconds);
/// ```
///
/// Values with unit [`unit::None`](`None`) can be converted to any other unit with a ratio of `1.0`.
///
/// ```
/// # use metrique_writer_core::{Convert, Observation, unit::{self, Second, Millisecond}};
/// let seconds = Observation::Floating(42.0);
/// let as_second = <unit::None as Convert<Second>>::convert(seconds);
/// assert_eq!(as_second, Observation::Floating(42.0));
///
/// // and also this:
/// let seconds = Observation::Floating(42.0);
/// let as_millisecond = <unit::None as Convert<Millisecond>>::convert(seconds);
/// assert_eq!(as_millisecond, Observation::Floating(42.0));
/// ```
pub trait Convert<U: UnitTag>: UnitTag {
    /// Ratio to convert from `Self` to `U`
    const RATIO: f64;

    /// Convert an [Observation] in units `Self` to an [Observation] in units `U`
    fn convert(observation: Observation) -> Observation {
        // Avoid any u64 => f64 conversions if the value doesn't change
        if Self::RATIO == 1.0 {
            return observation;
        }

        match observation {
            Observation::Unsigned(u) => Observation::Floating((u as f64) * Self::RATIO),
            Observation::Floating(f) => Observation::Floating(f * Self::RATIO),
            Observation::Repeated { total, occurrences } => Observation::Repeated {
                total: total * Self::RATIO,
                occurrences,
            },
        }
    }
}

macro_rules! unit_tag {
    ($struct:ident, $conversion:ident, $value:expr) => {
        #[doc = concat!("[`UnitTag`] type that can be used to tag a value with `", stringify!($value), "`.")]
        pub struct $struct;

        impl UnitTag for $struct {
            const UNIT: Unit = $value;
        }

        impl fmt::Debug for $struct {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&Self::UNIT, f)
            }
        }

        impl fmt::Display for $struct {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&Self::UNIT, f)
            }
        }

        #[doc = concat!(
            "Wrapper type that will cause the underlying unit be [`Convert::convert`]ed to `",
            stringify!($value),
            "` when written."
        )]
        pub type $conversion<V> = WithUnit<V, $struct>;
    };
}

// "Unitless" units

unit_tag!(None, AsNone, Unit::None);

impl<U: UnitTag> Convert<U> for None {
    const RATIO: f64 = 1.0;
}

unit_tag!(Count, AsCount, Unit::Count);
unit_tag!(Percent, AsPercent, Unit::Percent);

// Time units

trait TimeTag: UnitTag {
    const FROM_SECONDS: u64;
}

macro_rules! time_unit_tag {
    ($($struct:ident, $conversion:ident, $scale:ident;)*) => {
        $(
            unit_tag!($struct, $conversion, Unit::Second(NegativeScale::$scale));

            impl TimeTag for $struct {
                const FROM_SECONDS: u64 = NegativeScale::$scale.reduction_factor();
            }

            impl<U: TimeTag> Convert<U> for $struct {
                const RATIO: f64 = (U::FROM_SECONDS as f64)/(Self::FROM_SECONDS as f64);
            }
        )*
    };
}

time_unit_tag! {
    Second, AsSeconds, One;
    Millisecond, AsMilliseconds, Milli;
    Microsecond, AsMicroseconds, Micro;
}

// Bit units

trait BitTag: UnitTag {
    const FROM_BITS: u64;
}

macro_rules! bit_unit_tag {
    ($($struct:ident, $conversion:ident, $base:ident, $bits:expr, $scale:ident;)*) => {
        $(
            unit_tag!($struct, $conversion, Unit::$base(PositiveScale::$scale));

            impl BitTag for $struct {
                const FROM_BITS: u64 = $bits*PositiveScale::$scale.expansion_factor();
            }

            impl<U: BitTag> Convert<U> for $struct {
                const RATIO: f64 = (Self::FROM_BITS as f64)/(U::FROM_BITS as f64);
            }
        )*
    };
}

bit_unit_tag! {
    Byte, AsBytes, Byte, 8, One;
    Kilobyte, AsKilobytes, Byte, 8, Kilo;
    Megabyte, AsMegabytes, Byte, 8, Mega;
    Gigabyte, AsGigabytes, Byte, 8, Giga;
    Terabyte, AsTerabytes, Byte, 8, Tera;

    Bit, AsBits, Bit, 1, One;
    Kilobit, AsKilobits, Bit, 1, Kilo;
    Megabit, AsMegabits, Bit, 1, Mega;
    Gigabit, AsGigabits, Bit, 1, Giga;
    Terabit, AsTerabits, Bit, 1, Tera;

    BytePerSecond, AsBytesPerSecond, BytePerSecond, 8, One;
    KilobytePerSecond, AsKilobytesPerSecond, BytePerSecond, 8, Kilo;
    MegabytePerSecond, AsMegabytesPerSecond, BytePerSecond, 8, Mega;
    GigabytePerSecond, AsGigabytesPerSecond, BytePerSecond, 8, Giga;
    TerabytePerSecond, AsTerabytesPerSecond, BytePerSecond, 8, Tera;

    BitPerSecond, AsBitsPerSecond, BitPerSecond, 1, One;
    KilobitPerSecond, AsKilobitsPerSecond, BitPerSecond, 1, Kilo;
    MegabitPerSecond, AsMegabitsPerSecond, BitPerSecond, 1, Mega;
    GigabitPerSecond, AsGigabitsPerSecond, BitPerSecond, 1, Giga;
    TerabitPerSecond, AsTerabitsPerSecond, BitPerSecond, 1, Tera;
}

// Utilities to convert

/// Converts a value to the unit `U` in [`crate::Value::write()`].
///
/// Note that not all unit conversion are possible. `V` must have a value that implements [`Convert`] to `U`.
///
/// This can give a value with a [`None`] unit some more specific unit like [`Percent`], or change the scale that the
/// value is reported in, like reporting in [`Microsecond`]s rather than the default of [`Millisecond`]s for durations.
///
/// # Usage
///
/// For readability, prefer the `As{Unit}` type aliases, like [`AsSeconds<T>`](`AsSeconds`) rather than
/// `WithUnit<T, Second>`.
/// ```
/// # use metrique_writer::unit::{AsSeconds, AsBytes};
/// # use metrique_writer::Entry;
/// # use std::time::Duration;
///
/// #[derive(Entry)]
/// struct MyEntry {
///     my_timer: AsSeconds<Duration>,
///     request_size: AsBytes<u64>,
/// }
///
/// // `WithUnit` (and the aliases) implement `From`, initialize them like this:
/// MyEntry {
///     my_timer: Duration::from_secs(2).into(),
///     request_size: 2u64.into(),
/// };
/// ```
pub struct WithUnit<V, U> {
    value: V,
    _unit_tag: PhantomData<U>,
}

impl<V: MetricValue, U> From<V> for WithUnit<V, U> {
    fn from(value: V) -> Self {
        Self {
            value,
            _unit_tag: PhantomData,
        }
    }
}

impl<V, U> WithUnit<V, U> {
    /// Return the wrapped value
    pub fn into_inner(self) -> V {
        self.value
    }
}

// Delegate all of the usual traits to V so we can ignore the unit tag type

impl<V: Default + MetricValue, U> Default for WithUnit<V, U> {
    fn default() -> Self {
        Self {
            value: V::default(),
            _unit_tag: PhantomData,
        }
    }
}

impl<V, U> Deref for WithUnit<V, U> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<V, U> DerefMut for WithUnit<V, U> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<V: Clone, U> Clone for WithUnit<V, U> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            _unit_tag: PhantomData,
        }
    }
}

impl<V: Copy, U> Copy for WithUnit<V, U> {}

impl<V: PartialEq, U> PartialEq for WithUnit<V, U> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<V: Eq, U> Eq for WithUnit<V, U> {}

impl<V: PartialOrd, U> PartialOrd for WithUnit<V, U> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<V: Ord, U> Ord for WithUnit<V, U> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl<V: Hash, U: UnitTag> Hash for WithUnit<V, U> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
        U::UNIT.hash(state);
    }
}

impl<V: Debug, U: UnitTag> fmt::Debug for WithUnit<V, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WithUnit")
            .field("value", &self.value)
            .field("unit", &U::UNIT)
            .finish()
    }
}

impl<V: Display, U: UnitTag> fmt::Display for WithUnit<V, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.value, U::UNIT)
    }
}

impl<V: MetricValue, U: UnitTag> Value for WithUnit<V, U>
where
    V::Unit: Convert<U>,
{
    fn write(&self, writer: impl ValueWriter) {
        struct Wrapper<W, From, To> {
            writer: W,
            _convert: PhantomData<(From, To)>,
        }

        impl<W: ValueWriter, From: Convert<To>, To: UnitTag> ValueWriter for Wrapper<W, From, To> {
            fn string(self, _value: &str) {
                self.invalid("can't apply a unit to a string value");
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                flags: MetricFlags<'_>,
            ) {
                if unit != From::UNIT {
                    self.invalid(format!(
                        "value promised to write unit `{}` but wrote `{unit}` instead",
                        From::UNIT
                    ));
                } else {
                    self.writer.metric(
                        distribution.into_iter().map(<From as Convert<To>>::convert),
                        To::UNIT,
                        dimensions,
                        flags,
                    )
                }
            }

            fn error(self, error: ValidationError) {
                self.writer.error(error)
            }
        }

        self.value.write(Wrapper {
            writer,
            _convert: PhantomData::<(V::Unit, U)>,
        })
    }
}

impl<V: MetricValue, U: UnitTag> MetricValue for WithUnit<V, U>
where
    V::Unit: Convert<U>,
{
    type Unit = U;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::MetricValue;

    use super::*;

    #[test]
    fn conversion_ratios() {
        // None to anything should always be 1
        assert_eq!(<None as Convert<Millisecond>>::RATIO, 1.0);
        assert_eq!(<None as Convert<Bit>>::RATIO, 1.0);
        assert_eq!(<None as Convert<MegabytePerSecond>>::RATIO, 1.0);
        assert_eq!(<None as Convert<Count>>::RATIO, 1.0);
        assert_eq!(<None as Convert<None>>::RATIO, 1.0);

        // Time conversions
        assert_eq!(<Second as Convert<Millisecond>>::RATIO, 1_000.0);
        assert_eq!(<Millisecond as Convert<Second>>::RATIO, 1.0 / 1_000.0);

        // Bit conversions
        assert_eq!(<Byte as Convert<Bit>>::RATIO, 8.0);
        assert_eq!(<Bit as Convert<Byte>>::RATIO, 1.0 / 8.0);
        assert_eq!(<Megabyte as Convert<Gigabit>>::RATIO, 8.0 / 1_000.0);
        assert_eq!(<Gigabit as Convert<Megabyte>>::RATIO, 1_000.0 / 8.0);

        // Bps conversions
        assert_eq!(<BytePerSecond as Convert<BitPerSecond>>::RATIO, 8.0);
        assert_eq!(<BitPerSecond as Convert<BytePerSecond>>::RATIO, 1.0 / 8.0);
        assert_eq!(
            <MegabytePerSecond as Convert<GigabitPerSecond>>::RATIO,
            8.0 / 1_000.0
        );
        assert_eq!(
            <GigabitPerSecond as Convert<MegabytePerSecond>>::RATIO,
            1_000.0 / 8.0
        );
    }

    #[test]
    fn fail_if_value_didnt_write_expected_unit() {
        struct Writer;
        impl ValueWriter for Writer {
            fn string(self, value: &str) {
                panic!("shouldn't have written {value}");
            }

            fn metric<'a>(
                self,
                _distribution: impl IntoIterator<Item = Observation>,
                _unit: Unit,
                _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                panic!("shouldn't have emitted metric");
            }

            fn error(self, error: ValidationError) {
                assert!(
                    error.to_string().contains(
                        "value promised to write unit `Seconds` but wrote `Bytes` instead"
                    )
                );
            }
        }

        struct BadValue;

        impl MetricValue for BadValue {
            type Unit = Second;
        }

        impl Value for BadValue {
            fn write(&self, writer: impl ValueWriter) {
                writer.metric([], Byte::UNIT, [], MetricFlags::empty());
            }
        }

        AsMilliseconds::from(BadValue).write(Writer);
    }

    #[test]
    fn converts_observations_and_passes_through_rest() {
        struct Writer;
        impl ValueWriter for Writer {
            fn string(self, value: &str) {
                panic!("shouldn't have written {value}");
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                _flags: MetricFlags<'_>,
            ) {
                let distribution = distribution.into_iter().collect::<Vec<_>>();
                let dimensions = dimensions.into_iter().collect::<Vec<_>>();

                assert_eq!(distribution, &[Observation::Floating(0.042)]);
                assert_eq!(unit, Second::UNIT);
                assert_eq!(dimensions, &[("foo", "bar")]);
            }

            fn error(self, error: ValidationError) {
                panic!("unexpected error {error}");
            }
        }

        AsSeconds::from(Duration::from_millis(42))
            .with_dimension("foo", "bar")
            .write(Writer);
    }
}
