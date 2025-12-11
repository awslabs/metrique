// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    io, mem,
    time::{Duration, Instant},
};

use ahash::HashMap;
use metrique_writer_core::{Entry, IoStreamError, entry::SampleGroupElement, format::Format};
use rand::{Rng, RngCore, rngs::ThreadRng};
use smallvec::SmallVec;

use super::{DefaultRng, SampledFormat};

#[derive(Debug)]
/// A builder for [CongressSample]
pub struct CongressSampleBuilder {
    interval: Duration,
    target_observed: u32,
    validate_groups: bool,
}

impl Default for CongressSampleBuilder {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(15),
            target_observed: 15 * 100,
            validate_groups: cfg!(debug_assertions),
        }
    }
}

impl CongressSampleBuilder {
    /// Wrap the given [`Format`] that supports sampling with the congressional sampling behavior,
    /// using the [DefaultRng].
    pub fn build<F>(self, format: F) -> CongressSample<F> {
        Self::build_with_rng(self, format, Default::default())
    }

    /// Create a new [CongressSampleBuilder] that wraps a [`Format`], allowing
    /// you to specify the random number generator. This is useful if you
    /// want to manually seed the RNG to allow for deterministic tests,
    /// but normally you should be using the [DefaultRng] in production.
    pub fn build_with_rng<F, R>(self, format: F, rng: R) -> CongressSample<F, R> {
        CongressSample {
            format,
            rng,
            interval: self.interval,
            target_observed: self.target_observed,
            validate_groups: self.validate_groups,
            next_interval_start: Instant::now(),
            current_observed: 0,
            groups: Default::default(),
        }
    }

    /// Defines over what interval we compute the average rate of different sample groups.
    ///
    /// Intervals that are too short to include many events of each group will lead to less accurate sampling. Intervals
    /// that are too long will be slower to adjust to changes in event frequency.
    ///
    /// Defaults to 15 seconds.
    pub fn interval(mut self, interval: Duration) -> Self {
        assert!(interval > Duration::ZERO);
        self.interval = interval;
        self
    }

    /// Defines the (soft) maximum entries should be emitted per interval.
    ///
    /// This maximum only applies to the equilibrium case where the frequency of events by sample group have remained
    /// stable. If they change dramatically, the next few intervals may emit more entries.
    ///
    /// Defaults to 1,500 (or 100 per second in 15 second intervals).
    pub fn target_entries_per_interval(mut self, target: u32) -> Self {
        assert!(target > 0);
        self.target_observed = target;
        self
    }

    /// Panic if an entry's [`Entry::sample_group`] contains non-unique keys.
    pub fn validate_groups(mut self, validate: bool) -> Self {
        self.validate_groups = validate;
        self
    }
}

/// Tries to write at most *n* entries per second and uses a
/// [congressional sampling strategy](https://dl.acm.org/doi/abs/10.1145/335191.335450) to boost the accuracy of
/// low-frequency events.
pub struct CongressSample<F, R = DefaultRng<ThreadRng>> {
    format: F,
    rng: R,
    interval: Duration,
    target_observed: u32,
    validate_groups: bool,

    next_interval_start: Instant,
    current_observed: u32,
    groups: HashMap<Group, GroupState>,
}

type Group = SmallVec<[SampleGroupElement; 2]>;

impl<F: SampledFormat, R: RngCore> Format for CongressSample<F, R> {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        let mut group: Group = entry.sample_group().collect();
        group.sort_unstable();

        if self.validate_groups {
            for pair in group.windows(2) {
                assert!(
                    pair[0].0 != pair[1].0,
                    "duplicate group element name `{}`",
                    pair[0].0
                );
            }
        }

        let rate = self.sample_rate(group);
        if rate == 1.0 || self.rng.random::<f32>() <= rate {
            self.format.format_with_sample_rate(entry, output, rate)
        } else {
            Ok(())
        }
    }
}

impl<F, R> CongressSample<F, R> {
    /// Return a mutable reference to the inner [`Format`].
    ///
    /// This can be used to for example wrap `CongressSample` in something
    /// that bypasses the sampling for some types of entries.
    ///
    /// ```
    /// # use metrique_writer::format::{Format, FormatExt};
    /// # use metrique_writer::sample::{CongressSample, SampledFormatExt};
    /// # use metrique_writer::{Entry, EntryIoStream, EntryIoStreamExt};
    /// # use metrique_writer::IoStreamError;
    /// # use metrique_writer_format_emf::{Emf, SampledEmf};
    /// # use std::io;
    /// # use std::sync::Arc;
    /// # use std::sync::atomic::{self, AtomicBool};
    /// # use std::time::SystemTime;
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct MyMetrics {
    ///     #[entry(timestamp)]
    ///     start: SystemTime,
    ///     #[entry(sample_group)]
    ///     operation: &'static str,
    /// }
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct Globals {
    ///    az: String,
    /// }
    ///
    /// struct MyFormatter {
    ///     inner: CongressSample<SampledEmf>,
    ///     bypass_sampling: Arc<AtomicBool>,
    /// }
    ///
    /// impl Format for MyFormatter {
    ///     fn format(
    ///         &mut self,
    ///         entry: &impl Entry,
    ///         output: &mut impl io::Write,
    ///     ) -> Result<(), IoStreamError> {
    ///         if self.bypass_sampling.load(atomic::Ordering::Relaxed) {
    ///             self.inner.format_mut().format(entry, output)
    ///         } else {
    ///             self.inner.format(entry, output)
    ///         }
    ///     }
    /// }
    ///
    /// let bypass_sampling = Arc::new(AtomicBool::new(false));
    /// let format = MyFormatter {
    ///     // pick a very low fraction to see that this works
    ///     inner: Emf::all_validations("MyApp".into(), vec![vec![]])
    ///         .with_sampling()
    ///         .sample_by_congress_at_fixed_entries_per_second(1),
    ///     bypass_sampling: bypass_sampling.clone(),
    /// };
    ///
    /// let globals = Globals {
    ///     az: "us-east-1a".into(),
    /// };
    ///
    /// let mut output = vec![];
    /// let mut stream = format.output_to(&mut output).merge_globals(globals);
    ///
    /// // this is sampled with a probability and potentially dropped
    /// stream.next(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     operation: "WillBePotentiallyDropped",
    /// }).unwrap();
    ///
    /// // this bypasses sampling
    /// bypass_sampling.store(true, atomic::Ordering::Relaxed);
    /// stream.next(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     operation: "WillRemain",
    /// }).unwrap();
    /// ```
    ///
    /// [`Format`]: crate::format::Format
    pub fn format_mut(&mut self) -> &mut F {
        &mut self.format
    }

    fn sample_rate(&mut self, group: Group) -> f32 {
        let now = Instant::now();
        if now > self.next_interval_start {
            self.next_interval_start = now + self.interval;
            self.update_rates();
        }

        self.current_observed += 1;
        let state = self.groups.entry(group).or_insert_with(|| GroupState {
            sample_rate: 1.0,
            ..Default::default()
        });
        state.record_observation();
        state.sample_rate
    }

    // Pulled out as a pure fn that doesn't depend on the clock so we can unit test more easily.
    fn update_rates(&mut self) {
        self.groups.retain(|_, group| group.update_and_retain());

        let current_observed = mem::replace(&mut self.current_observed, 0) as f32;
        let target_observed = self.target_observed as f32;
        let flat_rate = target_observed / current_observed;
        let group_senate_size = target_observed / (self.groups.len() as f32);

        let mut congress_size = 0.0;
        for group in self.groups.values_mut() {
            let average = group.average_observed.current();
            let group_house_size = flat_rate * average;

            // Note this is not the same as group_house_size.max(average.min(senate))!
            group.size_in_congress = if group_house_size < group_senate_size {
                average.min(group_senate_size)
            } else {
                group_house_size
            };
            congress_size += group.size_in_congress;
        }

        if current_observed <= target_observed {
            for state in self.groups.values_mut() {
                state.sample_rate = 1.0;
            }
        } else {
            let scale_factor = target_observed / congress_size;
            for group in self.groups.values_mut() {
                let average = group.average_observed.current();
                group.sample_rate = if average <= 0.0 {
                    1.0
                } else {
                    (group.size_in_congress * scale_factor / average).min(1.0)
                };
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct GroupState {
    current_observed: u32,
    consecutive_no_observations: u8,
    average_observed: ExpMovingAverage,
    sample_rate: f32,
    size_in_congress: f32,
}

impl GroupState {
    fn record_observation(&mut self) {
        self.current_observed += 1;
    }

    fn update_and_retain(&mut self) -> bool {
        let current_observed = mem::replace(&mut self.current_observed, 0);
        if current_observed > 0 {
            self.average_observed.add_sample(current_observed as f32);
            self.consecutive_no_observations = 0;
            true
        } else if self.consecutive_no_observations >= NO_OBSERVATIONS_TTL {
            false
        } else {
            self.consecutive_no_observations += 1;
            true
        }
    }
}

const EXP_MOVING_AVERAGE_WINDOW: u8 = 16;
const NO_OBSERVATIONS_TTL: u8 = EXP_MOVING_AVERAGE_WINDOW / 2;

#[derive(Clone, Copy, Debug, Default)]
struct ExpMovingAverage {
    samples: u8,
    value: f32,
}

impl ExpMovingAverage {
    fn current(self) -> f32 {
        self.value
    }

    fn add_sample(&mut self, sample: f32) {
        self.samples = EXP_MOVING_AVERAGE_WINDOW.min(self.samples + 1);
        let decay = 1.0 / (self.samples as f32);
        self.value = decay * sample + (1.0 - decay) * self.value;
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use crate::{EntryWriter, ValueWriter, value::MetricFlags};

    use super::*;
    use assert_approx_eq::assert_approx_eq;

    #[test]
    fn group_is_not_retained_after_no_observations() {
        let mut state = GroupState {
            sample_rate: 1.0,
            ..Default::default()
        };
        for _ in 0..NO_OBSERVATIONS_TTL {
            assert!(state.update_and_retain());
        }
        assert!(!state.update_and_retain());
    }

    #[test]
    fn group_average_observed_converges() {
        let mut state = GroupState {
            sample_rate: 1.0,
            ..Default::default()
        };
        let observed = 100;
        for _ in 0..1_000 {
            for _ in 0..observed {
                state.record_observation();
            }
            assert!(state.update_and_retain());
        }
        assert_approx_eq!(state.average_observed.current(), observed as f32, 0.01);
    }

    #[test]
    fn does_not_sample_when_below_target() {
        let mut congress = CongressSampleBuilder::default()
            .target_entries_per_interval(100)
            .interval(Duration::from_secs(86400)) // trigger manually
            .build(TestFormat::default());

        // call the function since our coverage doesn't read doctests
        let _ = congress.format_mut();

        for _ in 0..100 {
            for (count, operation) in [(50, "A"), (40, "B"), (1, "C"), (5, "D")] {
                for _ in 0..count {
                    congress
                        .format(&TestEntry { operation }, &mut io::sink())
                        .unwrap();
                }
            }

            congress.update_rates();
            let in_interval = mem::take(&mut congress.format.entries);
            assert_eq!(in_interval.len(), 96);
            for (_entry, rate) in in_interval {
                assert_approx_eq!(rate, 1.0, 0.01);
            }
        }
    }

    #[test]
    fn does_sample_when_above_target() {
        let mut congress = CongressSampleBuilder::default()
            .target_entries_per_interval(100)
            .interval(Duration::from_secs(86400)) // trigger manually
            .build(TestFormat::default());

        for _ in 0..100 {
            congress.format.entries.clear();
            for (count, operation) in [(200, "A"), (200, "B")] {
                for _ in 0..count {
                    congress
                        .format(&TestEntry { operation }, &mut io::sink())
                        .unwrap();
                }
            }
            congress.update_rates();
        }

        let in_interval = mem::take(&mut congress.format.entries);
        for (_entry, rate) in in_interval {
            assert_approx_eq!(rate, 100.0 / (200.0 + 200.0), 0.01);
        }
    }

    #[test]
    fn several_groups() {
        let mut congress = CongressSampleBuilder::default()
            .target_entries_per_interval(200)
            .interval(Duration::from_secs(86400)) // trigger manually
            .build(TestFormat::default());

        for _ in 0..100 {
            congress.format.entries.clear();
            for (count, operation) in [(800, "A"), (50, "B"), (50, "C"), (50, "D"), (50, "E")] {
                for _ in 0..count {
                    congress
                        .format(&TestEntry { operation }, &mut io::sink())
                        .unwrap();
                }
            }
            congress.update_rates();
        }

        for (expected_rate, operation) in [
            (100.0 / 800.0, "A"),
            (25.0 / 50.0, "B"),
            (25.0 / 50.0, "C"),
            (25.0 / 50.0, "D"),
            (25.0 / 50.0, "E"),
        ] {
            let actual_rate =
                congress.groups[&[("operation".into(), operation.into())][..]].sample_rate;
            assert_approx_eq!(expected_rate, actual_rate, 0.01);
        }
    }

    // | SET | Unsampled | House | Senate | Congress | Final |
    // | A   | 72000     | 7488  | 7800   | 7800     | 7647  |
    // | B   | 78000     | 8112  | 7800   | 8112     | 7953  |
    #[test]
    fn test_update_rates() {
        let mut congress = CongressSampleBuilder::default()
            .target_entries_per_interval(15600)
            .interval(Duration::from_secs(60)) // trigger manually
            .build(TestFormat::default());

        congress.format.entries.clear();
        for (count, operation) in [(72000, "A"), (78000, "B")] {
            for _ in 0..count {
                congress
                    .format(&TestEntry { operation }, &mut io::sink())
                    .unwrap();
            }
        }
        congress.update_rates();

        for (expected_rate, operation) in [(7647.0 / 72000.0, "A"), (7953.0 / 78000.0, "B")] {
            let actual_rate =
                congress.groups[&[("operation".into(), operation.into())][..]].sample_rate;
            assert_approx_eq!(expected_rate, actual_rate, 0.01);
        }
    }

    // test that update_rates works when `congress_size <= target_observed`
    #[test]
    fn test_update_rates_current_observed_greater_than_target_observed() {
        let mut congress = CongressSampleBuilder::default()
            .target_entries_per_interval(200)
            .interval(Duration::from_secs(86400)) // trigger manually
            .build(TestFormat::default());

        congress.format.entries.clear();
        for (count, operation) in [(100, "A"), (100, "B")] {
            for _ in 0..count {
                congress
                    .format(&TestEntry { operation }, &mut io::sink())
                    .unwrap();
            }
        }
        congress.update_rates();

        congress.format.entries.clear();
        for (count, operation) in [(400, "A"), (200, "B")] {
            for _ in 0..count {
                congress
                    .format(&TestEntry { operation }, &mut io::sink())
                    .unwrap();
            }
        }
        congress.update_rates();

        for operation in ["A", "B"] {
            let actual_rate =
                congress.groups[&[("operation".into(), operation.into())][..]].sample_rate;
            assert!(actual_rate != 1.0);
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct TestEntry {
        operation: &'static str,
    }

    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("operation", &self.operation);
        }

        fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
            [("operation".into(), self.operation.into())].into_iter()
        }
    }

    #[derive(Default)]
    struct TestFormat {
        entries: Vec<(String, f32)>,
    }

    impl Format for TestFormat {
        fn format(
            &mut self,
            _entry: &impl Entry,
            _output: &mut impl io::Write,
        ) -> Result<(), IoStreamError> {
            unreachable!("should be using sampled format fns")
        }
    }

    impl SampledFormat for TestFormat {
        fn format_with_sample_rate(
            &mut self,
            entry: &impl Entry,
            _output: &mut impl io::Write,
            rate: f32,
        ) -> Result<(), IoStreamError> {
            struct Writer<'a> {
                format: &'a mut TestFormat,
                rate: f32,
            }

            impl<'a> EntryWriter<'a> for Writer<'_> {
                fn timestamp(&mut self, _timestamp: std::time::SystemTime) {
                    unreachable!()
                }

                fn value(
                    &mut self,
                    name: impl Into<Cow<'a, str>>,
                    value: &(impl crate::Value + ?Sized),
                ) {
                    assert_eq!(name.into(), "operation");
                    value.write(self);
                }

                fn config(&mut self, _config: &'a dyn crate::EntryConfig) {}
            }

            impl ValueWriter for &mut Writer<'_> {
                fn string(self, value: &str) {
                    self.format.entries.push((value.to_owned(), self.rate));
                }

                fn metric<'a>(
                    self,
                    _distribution: impl IntoIterator<Item = crate::Observation>,
                    _unit: crate::Unit,
                    _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                    _flags: MetricFlags<'_>,
                ) {
                    unreachable!()
                }

                fn error(self, _error: crate::ValidationError) {
                    unreachable!()
                }
            }

            entry.write(&mut Writer { format: self, rate });

            Ok(())
        }
    }
}
