// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Applying a [`Quantizer`] to values as they are written.

use std::ops::{Deref, DerefMut};

use smallvec::SmallVec;

use crate::{
    MetricFlags, Observation, Unit, ValidationError, Value, ValueWriter,
    quantize::{Quantizer, QuantizerSource},
    value::{MetricValue, VALUES_INLINE_CAPACITY},
};

/// Reduce the precision of a single [`Observation`].
///
/// [`Observation::Unsigned`] goes through [`Quantizer::quantize_u64`] and
/// [`Observation::Floating`] through [`Quantizer::quantize_f64`].
///
/// For [`Observation::Repeated`], only `total` is quantized; `occurrences` is passed through
/// untouched. `occurrences` is a count of how many measurements were summed into `total`, and
/// formats divide by it to recover an average. Quantizing it would corrupt that average by an
/// amount unrelated to the quantizer's error bound, so it is always left exact.
///
/// ```
/// use metrique_writer_core::Observation;
/// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
/// use metrique_writer_core::value::quantize_observation;
///
/// let quantizer = Quantizer::new(SignificantBits::new(4).unwrap(), Rounding::Floor);
///
/// assert_eq!(
///     quantize_observation(Observation::Unsigned(1000), quantizer),
///     Observation::Unsigned(960),
/// );
///
/// // `occurrences` survives; only the total is reduced.
/// assert_eq!(
///     quantize_observation(
///         Observation::Repeated { total: 1000.0, occurrences: 7 },
///         quantizer,
///     ),
///     Observation::Repeated { total: 960.0, occurrences: 7 },
/// );
/// ```
pub fn quantize_observation(observation: Observation, quantizer: Quantizer) -> Observation {
    // Matched exhaustively on purpose. `Observation` is `#[non_exhaustive]`, but within this
    // crate that does not force a wildcard arm, and a wildcard would let a future numeric
    // variant slip through unquantized without anyone noticing. Breaking the build is the
    // outcome we want if a variant is added.
    match observation {
        Observation::Unsigned(value) => Observation::Unsigned(quantizer.quantize_u64(value)),
        Observation::Floating(value) => Observation::Floating(quantizer.quantize_f64(value)),
        Observation::Repeated { total, occurrences } => Observation::Repeated {
            total: quantizer.quantize_f64(total),
            occurrences,
        },
    }
}

/// A [`ValueWriter`] that reduces the precision of every observation passed through it.
///
/// This is the mechanism behind [`Quantized`](super::Quantized) and is public so that
/// formats, sinks, and other writer wrappers can apply the same reduction. String properties,
/// units, dimensions, and validation errors are forwarded unchanged; only numeric
/// observations are altered.
///
/// # Examples
///
/// ```
/// use metrique_writer_core::{Observation, Unit, Value, ValueWriter, MetricFlags};
/// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
/// use metrique_writer_core::value::QuantizingValueWriter;
///
/// // A writer that records whatever it is handed.
/// struct Recorder<'a>(&'a mut Vec<u64>);
///
/// impl ValueWriter for Recorder<'_> {
///     fn string(self, _: &str) {}
///     fn metric<'a>(
///         self,
///         distribution: impl IntoIterator<Item = Observation>,
///         _: Unit,
///         _: impl IntoIterator<Item = (&'a str, &'a str)>,
///         _: MetricFlags<'_>,
///     ) {
///         for observation in distribution {
///             if let Observation::Unsigned(value) = observation {
///                 self.0.push(value);
///             }
///         }
///     }
///     fn error(self, _: metrique_writer_core::ValidationError) {}
/// }
///
/// let quantizer = Quantizer::new(SignificantBits::new(4).unwrap(), Rounding::Floor);
///
/// let mut recorded = Vec::new();
/// 1000u64.write(QuantizingValueWriter::new(Recorder(&mut recorded), quantizer));
/// assert_eq!(recorded, [960]);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct QuantizingValueWriter<W> {
    inner: W,
    quantizer: Quantizer,
}

impl<W> QuantizingValueWriter<W> {
    /// Wrap `inner` so that observations written through it are reduced by `quantizer`.
    pub fn new(inner: W, quantizer: Quantizer) -> Self {
        Self { inner, quantizer }
    }

    /// The quantizer being applied.
    pub fn quantizer(&self) -> Quantizer {
        self.quantizer
    }
}

impl<W: ValueWriter> ValueWriter for QuantizingValueWriter<W> {
    fn string(self, value: &str) {
        // String properties have no numeric precision to reduce.
        self.inner.string(value)
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        flags: MetricFlags<'_>,
    ) {
        let quantizer = self.quantizer;
        // Mapped lazily, so a distribution of any length is quantized without allocating.
        self.inner.metric(
            distribution
                .into_iter()
                .map(move |observation| quantize_observation(observation, quantizer)),
            unit,
            dimensions,
            flags,
        )
    }

    fn error(self, error: ValidationError) {
        self.inner.error(error)
    }

    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        // Forwarded rather than left to the default implementation, which would comma-join the
        // elements into a single string and never call `metric()` on them. Each element is
        // re-wrapped so its own observations are quantized too.
        let quantizer = self.quantizer;
        let wrapped: SmallVec<[QuantizingValue<&'a V>; VALUES_INLINE_CAPACITY]> = values
            .into_iter()
            .map(|value| QuantizingValue { value, quantizer })
            .collect();
        self.inner.values(wrapped.iter())
    }
}

/// Pairs a value with a quantizer so that lists can be forwarded element by element.
///
/// Kept private: [`Quantized`] is the public way to attach a quantizer to a value, and this
/// exists only so [`QuantizingValueWriter::values`] has something to hand to the writer it
/// wraps.
#[derive(Debug, Clone, Copy)]
struct QuantizingValue<V> {
    value: V,
    quantizer: Quantizer,
}

impl<V: Value> Value for QuantizingValue<V> {
    const SHAPE: crate::descriptor::FieldShape<'static> = V::SHAPE;
    const UNIT: crate::Unit = V::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        self.value
            .write(QuantizingValueWriter::new(writer, self.quantizer))
    }
}

/// Reduces the precision of a [`Value`] when it is written.
///
/// The wrapped value is emitted with the leading `N` significant bits of each observation
/// retained and the rest discarded. Everything else about the value — its unit, its
/// dimensions, its [`Value::SHAPE`] — is preserved, so wrapping a field changes only the
/// numbers.
///
/// The quantizer can be supplied either as data or as a type, via [`QuantizerSource`]:
///
/// - `Quantized<V>` holds a [`Quantizer`] as a field, which is what
///   [`MetricValue::quantized`] produces. Use this when the settings come from configuration.
/// - `Quantized<V, Bits<N>>` carries the settings in the type, costs no space at runtime, and
///   rejects an out-of-range `N` at compile time.
///
/// # Composing with units
///
/// Order matters when combining with [`MetricValue::with_unit`]. Convert first, then quantize:
///
/// ```
/// use std::time::Duration;
/// use metrique_writer_core::quantize::{Rounding, SignificantBits};
/// use metrique_writer_core::unit::Microsecond;
/// use metrique_writer_core::value::MetricValue;
///
/// let bits = SignificantBits::new(8).unwrap();
///
/// // Correct: the error bound applies to the value as emitted, in microseconds.
/// let value = Duration::from_millis(3).with_unit::<Microsecond>().quantized(bits, Rounding::Midpoint);
/// # let _ = value;
/// ```
///
/// Quantizing before converting would apply the bound in the source unit and then scale the
/// result, leaving the emitted number off the target unit's lattice.
///
/// # Examples
///
/// Attaching a quantizer as data:
///
/// ```
/// use metrique_writer_core::quantize::{Rounding, SignificantBits};
/// use metrique_writer_core::value::MetricValue;
///
/// let bits = SignificantBits::new(4).unwrap();
/// let quantized = 1000u64.quantized(bits, Rounding::Floor);
///
/// // `Deref` reaches the value that was wrapped, unchanged.
/// assert_eq!(*quantized, 1000);
/// ```
///
/// Attaching it as a type, which is what a field declaration usually wants:
///
/// ```
/// use metrique_writer_core::quantize::{Bits, rounding};
/// use metrique_writer_core::value::Quantized;
///
/// // 8 significant bits with the default midpoint rounding.
/// let latency: Quantized<u64, Bits<8>> = 1000u64.into();
///
/// // 11 significant bits, never overstating.
/// let conservative: Quantized<u64, Bits<11, rounding::Floor>> = 1000u64.into();
/// # let _ = (latency, conservative);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Quantized<V, Q = Quantizer> {
    value: V,
    quantizer: Q,
}

impl<V> Quantized<V, Quantizer> {
    /// Wrap `value` so that it is reduced by `quantizer` when written.
    pub fn new(value: V, quantizer: Quantizer) -> Self {
        Self { value, quantizer }
    }
}

impl<V, Q> Quantized<V, Q> {
    /// Wrap `value` with a quantizer supplied as either data or a type.
    pub fn with_source(value: V, quantizer: Q) -> Self {
        Self { value, quantizer }
    }

    /// The quantizer settings attached to this value.
    pub fn source(&self) -> &Q {
        &self.quantizer
    }

    /// Return the value that was wrapped, discarding the quantizer.
    pub fn into_inner(self) -> V {
        self.value
    }

    /// Map the value within this [`Quantized`], keeping the same quantizer.
    pub fn map_value<U>(self, f: impl FnOnce(V) -> U) -> Quantized<U, Q> {
        Quantized {
            value: f(self.value),
            quantizer: self.quantizer,
        }
    }
}

impl<V, Q: Default> From<V> for Quantized<V, Q> {
    fn from(value: V) -> Self {
        Self {
            value,
            quantizer: Q::default(),
        }
    }
}

impl<V, Q> Deref for Quantized<V, Q> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<V, Q> DerefMut for Quantized<V, Q> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<V: Value, Q: QuantizerSource> Value for Quantized<V, Q> {
    const SHAPE: crate::descriptor::FieldShape<'static> = V::SHAPE;
    const UNIT: crate::Unit = V::UNIT;

    fn write(&self, writer: impl ValueWriter) {
        self.value.write(QuantizingValueWriter::new(
            writer,
            self.quantizer.quantizer(),
        ))
    }
}

impl<V: MetricValue, Q: QuantizerSource> MetricValue for Quantized<V, Q> {
    type Unit = V::Unit;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantize::{Rounding, SignificantBits};

    fn quantizer(bits: u8, rounding: Rounding) -> Quantizer {
        Quantizer::new(SignificantBits::new(bits).unwrap(), rounding)
    }

    #[derive(Debug, PartialEq)]
    enum Event {
        String(String),
        ValuesStart,
        Metric {
            distribution: Vec<Observation>,
            unit: Unit,
            dimensions: Vec<(String, String)>,
        },
        Error(String),
    }

    struct Recorder<'a>(&'a mut Vec<Event>);

    impl ValueWriter for Recorder<'_> {
        fn string(self, value: &str) {
            self.0.push(Event::String(value.to_string()));
        }

        fn metric<'a>(
            self,
            distribution: impl IntoIterator<Item = Observation>,
            unit: Unit,
            dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
            _flags: MetricFlags<'_>,
        ) {
            self.0.push(Event::Metric {
                distribution: distribution.into_iter().collect(),
                unit,
                dimensions: dimensions
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            });
        }

        fn error(self, error: ValidationError) {
            self.0.push(Event::Error(error.to_string()));
        }

        // Overridden so a forwarded `values()` call is distinguishable from the default
        // comma-joined `string()` fallback.
        fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
            self.0.push(Event::ValuesStart);
            for value in values {
                value.write(Recorder(self.0));
            }
        }
    }

    fn record(value: &(impl Value + ?Sized), quantizer: Quantizer) -> Vec<Event> {
        let mut events = Vec::new();
        value.write(QuantizingValueWriter::new(Recorder(&mut events), quantizer));
        events
    }

    /// Record a value that already carries its own quantizer, with no extra wrapping.
    fn record_plain(value: &(impl Value + ?Sized)) -> Vec<Event> {
        let mut events = Vec::new();
        value.write(Recorder(&mut events));
        events
    }

    fn distribution(events: &[Event]) -> &[Observation] {
        match &events[0] {
            Event::Metric { distribution, .. } => distribution,
            other => panic!("expected a metric, got {other:?}"),
        }
    }

    #[test]
    fn quantizes_unsigned_observations() {
        let events = record(&1000u64, quantizer(4, Rounding::Floor));
        assert_eq!(distribution(&events), &[Observation::Unsigned(960)]);
    }

    #[test]
    fn quantizes_floating_observations() {
        let events = record(&1000.0f64, quantizer(4, Rounding::Floor));
        assert_eq!(distribution(&events), &[Observation::Floating(960.0)]);
    }

    #[test]
    fn quantizes_repeated_total_but_not_occurrences() {
        let observation = Observation::Repeated {
            total: 1000.0,
            occurrences: 7,
        };
        let events = record(&observation, quantizer(4, Rounding::Floor));
        assert_eq!(
            distribution(&events),
            &[Observation::Repeated {
                total: 960.0,
                occurrences: 7,
            }]
        );
    }

    #[test]
    fn repeated_occurrences_survive_every_mode_and_width() {
        for bits in 1..=52u8 {
            for rounding in [Rounding::Floor, Rounding::Ceil, Rounding::Midpoint] {
                let observation = Observation::Repeated {
                    total: 123_456.0,
                    occurrences: 999,
                };
                let events = record(&observation, quantizer(bits, rounding));
                match &distribution(&events)[0] {
                    Observation::Repeated { occurrences, .. } => {
                        assert_eq!(*occurrences, 999, "bits={bits} rounding={rounding}")
                    }
                    other => panic!("expected Repeated, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn leaves_strings_untouched() {
        let events = record("1000", quantizer(4, Rounding::Floor));
        assert_eq!(events, [Event::String("1000".to_string())]);
    }

    #[test]
    fn forwards_units_and_dimensions_unchanged() {
        use crate::unit::{Millisecond, UnitTag as _};
        use crate::value::MetricValue;

        let value = std::time::Duration::from_millis(1000).with_dimension("Operation", "Ducks");
        let events = record(&value, quantizer(4, Rounding::Floor));

        match &events[0] {
            Event::Metric {
                distribution,
                unit,
                dimensions,
            } => {
                assert_eq!(distribution, &[Observation::Floating(960.0)]);
                assert_eq!(*unit, Millisecond::UNIT);
                assert_eq!(
                    dimensions,
                    &[("Operation".to_string(), "Ducks".to_string())]
                );
            }
            other => panic!("expected a metric, got {other:?}"),
        }
    }

    #[test]
    fn forwards_errors_unchanged() {
        let mut events = Vec::new();
        QuantizingValueWriter::new(Recorder(&mut events), quantizer(4, Rounding::Floor))
            .error(ValidationError::invalid("something was wrong"));

        assert_eq!(
            events,
            [Event::Error(
                ValidationError::invalid("something was wrong").to_string()
            )]
        );
    }

    #[test]
    fn forwards_invalid_reason_unchanged() {
        let mut events = Vec::new();
        QuantizingValueWriter::new(Recorder(&mut events), quantizer(4, Rounding::Floor))
            .invalid("bad value");

        assert!(
            matches!(events.as_slice(), [Event::Error(message)] if message.contains("bad value")),
            "expected the reason to survive, got {events:?}"
        );
    }

    #[test]
    fn non_finite_observations_pass_through_unchanged() {
        // The `f64` value impl writes `NaN` and the infinities as observations and leaves it to
        // the format to validate them. Quantizing must not disturb them, so that whatever
        // diagnostic the format would have produced is unchanged.
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let events = record(&value, quantizer(4, Rounding::Floor));
            match &distribution(&events)[0] {
                Observation::Floating(written) => {
                    assert_eq!(
                        written.to_bits(),
                        value.to_bits(),
                        "expected {value} to pass through"
                    );
                }
                other => panic!("expected a floating observation, got {other:?}"),
            }
        }
    }

    #[test]
    fn forwards_values_as_a_native_list_with_each_element_quantized() {
        let events = record(&vec![1000u64, 2000u64], quantizer(4, Rounding::Floor));

        assert_eq!(
            events,
            [
                // `ValuesStart` proves `values()` was forwarded rather than falling back to
                // the comma-joined string representation.
                Event::ValuesStart,
                Event::Metric {
                    distribution: vec![Observation::Unsigned(960)],
                    unit: Unit::None,
                    dimensions: vec![],
                },
                Event::Metric {
                    distribution: vec![Observation::Unsigned(1920)],
                    unit: Unit::None,
                    dimensions: vec![],
                },
            ]
        );
    }

    #[test]
    fn forwards_empty_lists() {
        let events = record(&Vec::<u64>::new(), quantizer(4, Rounding::Floor));
        assert_eq!(events, [Event::ValuesStart]);
    }

    #[test]
    fn nested_lists_quantize_every_element() {
        let nested = vec![vec![1000u64, 2000u64], vec![3000u64]];
        let events = record(&nested, quantizer(4, Rounding::Floor));

        let totals: Vec<u64> = events
            .iter()
            .filter_map(|event| match event {
                Event::Metric { distribution, .. } => match distribution[0] {
                    Observation::Unsigned(value) => Some(value),
                    _ => None,
                },
                _ => None,
            })
            .collect();

        assert_eq!(totals, [960, 1920, 2816]);
    }

    #[test]
    fn passes_through_values_that_write_nothing() {
        let events = record(&Option::<u64>::None, quantizer(4, Rounding::Floor));
        assert_eq!(events, []);
    }

    #[test]
    fn quantizer_is_readable() {
        let quantizer = quantizer(11, Rounding::Ceil);
        let writer = QuantizingValueWriter::new((), quantizer);
        assert_eq!(writer.quantizer(), quantizer);
    }

    #[test]
    fn multi_observation_distributions_are_all_quantized() {
        // A value that writes several observations at once.
        let observations = vec![
            Observation::Unsigned(1000),
            Observation::Unsigned(2000),
            Observation::Floating(3000.0),
        ];
        let mut events = Vec::new();
        QuantizingValueWriter::new(Recorder(&mut events), quantizer(4, Rounding::Floor)).metric(
            observations,
            Unit::None,
            [],
            MetricFlags::empty(),
        );

        assert_eq!(
            distribution(&events),
            &[
                Observation::Unsigned(960),
                Observation::Unsigned(1920),
                Observation::Floating(2816.0),
            ]
        );
    }

    // ---------------------------------------------------------------------------------
    // Quantized
    // ---------------------------------------------------------------------------------

    #[test]
    fn quantized_wrapper_reduces_the_value() {
        let bits = SignificantBits::new(4).unwrap();
        let value = 1000u64.quantized(bits, Rounding::Floor);
        let events = record_plain(&value);
        assert_eq!(distribution(&events), &[Observation::Unsigned(960)]);
    }

    #[test]
    fn quantized_derefs_to_the_unmodified_value() {
        let bits = SignificantBits::new(4).unwrap();
        let mut value = 1000u64.quantized(bits, Rounding::Floor);

        // `Deref` and `DerefMut` see the true value, not the quantized one.
        assert_eq!(*value, 1000);
        *value += 1;
        assert_eq!(*value, 1001);
        assert_eq!(value.into_inner(), 1001);
    }

    #[test]
    fn quantized_from_type_level_bits() {
        use crate::quantize::{Bits, rounding as tag};

        let midpoint: Quantized<u64, Bits<4>> = 1000u64.into();
        assert_eq!(
            distribution(&record_plain(&midpoint)),
            &[Observation::Unsigned(992)]
        );

        let floor: Quantized<u64, Bits<4, tag::Floor>> = 1000u64.into();
        assert_eq!(
            distribution(&record_plain(&floor)),
            &[Observation::Unsigned(960)]
        );

        let ceil: Quantized<u64, Bits<4, tag::Ceil>> = 1000u64.into();
        assert_eq!(
            distribution(&record_plain(&ceil)),
            &[Observation::Unsigned(1024)]
        );
    }

    #[test]
    fn type_level_bits_are_zero_sized() {
        use crate::quantize::Bits;
        // The whole point of the type-level form: the settings cost no space.
        assert_eq!(
            std::mem::size_of::<Quantized<u64, Bits<8>>>(),
            std::mem::size_of::<u64>()
        );
    }

    #[test]
    fn quantized_preserves_shape_and_unit() {
        use crate::quantize::Bits;
        use std::time::Duration;

        assert_eq!(
            <Quantized<Duration> as Value>::UNIT,
            <Duration as Value>::UNIT
        );
        assert_eq!(
            <Quantized<u64, Bits<8>> as Value>::UNIT,
            <u64 as Value>::UNIT
        );

        // `FieldShape` is not `PartialEq`, so compare the rendered form.
        assert_eq!(
            format!("{:?}", <Quantized<Duration> as Value>::SHAPE),
            format!("{:?}", <Duration as Value>::SHAPE)
        );
        assert_eq!(
            format!("{:?}", <Quantized<u64, Bits<8>> as Value>::SHAPE),
            format!("{:?}", <u64 as Value>::SHAPE)
        );
    }

    #[test]
    fn quantized_map_value_keeps_the_quantizer() {
        let bits = SignificantBits::new(4).unwrap();
        let value = 1000u64.quantized(bits, Rounding::Floor);
        let mapped = value.map_value(|v| v * 2);

        assert_eq!(*mapped, 2000);
        assert_eq!(
            distribution(&record_plain(&mapped)),
            &[Observation::Unsigned(1920)]
        );
    }

    #[test]
    fn converting_the_unit_before_quantizing_applies_the_bound_in_that_unit() {
        use crate::unit::{Microsecond, UnitTag as _};
        use std::time::Duration;

        let bits = SignificantBits::new(8).unwrap();

        // Convert first: 3ms becomes 3000us, then quantizing at 8 bits lands on 2992.
        let converted_then_quantized = Duration::from_millis(3)
            .with_unit::<Microsecond>()
            .quantized(bits, Rounding::Floor);
        let events = record_plain(&converted_then_quantized);
        match &events[0] {
            Event::Metric {
                distribution, unit, ..
            } => {
                assert_eq!(distribution, &[Observation::Floating(2992.0)]);
                assert_eq!(*unit, Microsecond::UNIT);
            }
            other => panic!("expected a metric, got {other:?}"),
        }

        // Quantize first: 3 (milliseconds) already fits in 8 bits, so nothing is discarded and
        // the conversion then yields an unreduced 3000us. This is why the docs tell callers to
        // convert before quantizing.
        let quantized_then_converted = Duration::from_millis(3)
            .quantized(bits, Rounding::Floor)
            .with_unit::<Microsecond>();
        let events = record_plain(&quantized_then_converted);
        assert_eq!(
            distribution(&events),
            &[Observation::Floating(3000.0)],
            "quantizing before converting should leave the value unreduced"
        );
    }

    #[test]
    fn quantized_composes_with_dimensions() {
        use crate::value::WithDimension;

        let bits = SignificantBits::new(4).unwrap();
        let value = WithDimension::new(1000u64.quantized(bits, Rounding::Floor), "Op", "Count");
        let events = record_plain(&value);

        match &events[0] {
            Event::Metric {
                distribution,
                dimensions,
                ..
            } => {
                assert_eq!(distribution, &[Observation::Unsigned(960)]);
                assert_eq!(dimensions, &[("Op".to_string(), "Count".to_string())]);
            }
            other => panic!("expected a metric, got {other:?}"),
        }
    }

    #[test]
    fn quantized_lists_are_forwarded_natively() {
        // `Vec` is a `Value` but not a `MetricValue`, so it has no `.quantized()` builder;
        // wrap it directly.
        let bits = SignificantBits::new(4).unwrap();
        let value = Quantized::new(
            vec![1000u64, 2000u64],
            Quantizer::new(bits, Rounding::Floor),
        );
        let events = record_plain(&value);

        assert_eq!(
            events,
            [
                Event::ValuesStart,
                Event::Metric {
                    distribution: vec![Observation::Unsigned(960)],
                    unit: Unit::None,
                    dimensions: vec![],
                },
                Event::Metric {
                    distribution: vec![Observation::Unsigned(1920)],
                    unit: Unit::None,
                    dimensions: vec![],
                },
            ]
        );
    }

    #[test]
    fn quantized_option_skips_none() {
        let bits = SignificantBits::new(4).unwrap();
        assert_eq!(
            record_plain(&Some(1000u64).quantized(bits, Rounding::Floor)).len(),
            1
        );
        assert_eq!(
            record_plain(&None::<u64>.quantized(bits, Rounding::Floor)),
            []
        );
    }

    #[test]
    fn quantized_in_an_entry_leaves_other_fields_alone() {
        use crate::quantize::{Bits, rounding as tag};
        use crate::{Entry, EntryConfig, EntryWriter};
        use std::borrow::Cow;
        use std::time::SystemTime;

        // Written by hand rather than with `derive(Entry)`: this crate's own lib tests pull in
        // a second copy of `metrique-writer-core` through the `metrique-writer` dev-dependency,
        // so the derive resolves `Value` to the other copy and cannot see impls defined here.
        // `metrique-writer/tests/quantize.rs` covers the derive path, where only one copy
        // exists.
        struct TestEntry {
            quantized_latency: Quantized<u64, Bits<4, tag::Floor>>,
            exact_latency: u64,
            request_count: u64,
        }

        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("QuantizedLatency", &self.quantized_latency);
                writer.value("ExactLatency", &self.exact_latency);
                writer.value("RequestCount", &self.request_count);
            }
        }

        /// Collects each field's name and the single unsigned observation it wrote.
        struct Collector<'a>(&'a mut Vec<(String, u64)>, bool);

        impl<'a> EntryWriter<'a> for Collector<'_> {
            fn timestamp(&mut self, _timestamp: SystemTime) {
                self.1 = true;
            }

            fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
                let mut events = Vec::new();
                value.write(Recorder(&mut events));
                if let Event::Metric { distribution, .. } = &events[0]
                    && let Observation::Unsigned(written) = distribution[0]
                {
                    self.0.push((name.into().into_owned(), written));
                }
            }

            fn config(&mut self, _config: &'a dyn EntryConfig) {}
        }

        let entry = TestEntry {
            quantized_latency: 1000u64.into(),
            exact_latency: 1000,
            request_count: 1000,
        };

        let mut fields = Vec::new();
        let mut collector = Collector(&mut fields, false);
        entry.write(&mut collector);
        let saw_timestamp = collector.1;

        assert!(saw_timestamp, "the timestamp should still be written");
        assert_eq!(
            fields,
            [
                // Only the quantized field moved.
                ("QuantizedLatency".to_string(), 960),
                ("ExactLatency".to_string(), 1000),
                ("RequestCount".to_string(), 1000),
            ]
        );
    }

    #[test]
    fn quantized_source_is_readable() {
        let bits = SignificantBits::new(11).unwrap();
        let expected = Quantizer::new(bits, Rounding::Ceil);
        let value = 1u64.quantized(bits, Rounding::Ceil);
        assert_eq!(*value.source(), expected);
    }
}
