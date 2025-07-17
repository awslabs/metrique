// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains samplers that can be used to sample metrics to avoid
//! performance impact when the metric rate is high.
//!
//! The samplers in this module upweight sampled metrics, to ensure
//! that the aggregate statistics are not biased by sampling (for
//! example, if a datapoint is sampled with probability 10%, the
//! samplers will give it a weight of 10x - of course, the mechanism
//! of weighting entries is format-specific).
//!
//! Currently, contains the following samplers:
//!
//! 1. [FixedFractionSample], which samples metrics by a fixed sample.
//! 2. [CongressSample], which maintains a bounded rate of metric emission,
//!    and also tries to ensure that a reasonable amount of entries for
//!    every [sample group] is sampled.
//!
//! [sample group]: Entry::sample_group
//!
//! See the [SampledFormat] and [SampledFormatExt] traits for more details.

use std::{io, marker::PhantomData, time::Duration};

use metrique_writer_core::{Entry, IoStreamError, format::Format};
use rand::{Rng, RngCore, rngs::ThreadRng};

pub use metrique_writer_core::sample::SampledFormat;

mod congress;
pub use congress::{CongressSample, CongressSampleBuilder};

/// Utility wrapper to impl [`RngCore`] from a stateless random number generator that impls [`Default`], like
/// [`ThreadRng`].
#[derive(Default)]
// Note: we use PhantomData of fn() -> R instead of just R to avoid requiring Send bounds on R. This better
// reflects the actual usage R::default().next(), where we always create and drop a temporary R.
pub struct DefaultRng<R>(PhantomData<fn() -> R>);

impl<R: RngCore + Default> RngCore for DefaultRng<R> {
    fn next_u32(&mut self) -> u32 {
        R::default().next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        R::default().next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        R::default().fill_bytes(dest)
    }
}

/// Extension trait for SampledFormat
pub trait SampledFormatExt: SampledFormat {
    /// Discard all but `sample_rate` fraction of entries at random.
    ///
    /// A sample rate of 0.1 will reduce the formatting load by 10x but will come at the cost of lower accuracy. This is
    /// especially noticeable if metrics about different kinds of events are merged into the same format output (e.g.
    /// metrics from different APIs or different results). For a more accurate sample of heterogeneous events, see
    /// [`SampledFormat::sample_by_congress_at_fixed_entries_per_second`].
    fn sample_by_fixed_fraction(self, sample_rate: f32) -> FixedFractionSample<Self>
    where
        Self: Sized,
    {
        FixedFractionSample::new(self, sample_rate)
    }

    /// Tries to write at most *n* entries per second and uses a
    /// [congressional sampling strategy](https://dl.acm.org/doi/abs/10.1145/335191.335450) to boost the accuracy of
    /// low-frequency events.
    ///
    /// See [`CongressSample`].
    fn sample_by_congress_at_fixed_entries_per_second(
        self,
        target_entries_per_second: u32,
    ) -> CongressSample<Self>
    where
        Self: Sized,
    {
        CongressSampleBuilder::default()
            .interval(Duration::from_secs(15))
            .target_entries_per_interval(
                target_entries_per_second
                    .checked_mul(15)
                    .expect("target entries too large"),
            )
            .build(self)
    }
}

impl<T: SampledFormat + ?Sized> SampledFormatExt for T {}

/// See [`SampledFormat::sample_by_fixed_fraction`].
pub struct FixedFractionSample<F, R = DefaultRng<ThreadRng>> {
    format: F,
    rate: f32,
    rng: R,
}

impl<F> FixedFractionSample<F> {
    /// Create a new [`SampledFormat`] from `format` that will emit events at a fixed randomly sampled `rate`.
    ///
    /// Uses the default [`ThreadRng`] for sampling.
    pub fn new(format: F, rate: f32) -> Self {
        Self::with_rng(format, rate, Default::default())
    }

    /// Return a mutable reference to the inner [`Format`].
    ///
    /// This can be used to for example wrap `FixedFractionSample` in something
    /// that bypasses the sampling for some types of entries.
    ///
    /// ```
    /// # use metrique_writer::format::{Format, FormatExt};
    /// # use metrique_writer::sample::{FixedFractionSample, SampledFormat, SampledFormatExt};
    /// # use metrique_writer::{Entry, EntryIoStream, EntryIoStreamExt, IoStreamError};
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
    ///     operation: String,
    /// }
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct Globals {
    ///    az: String,
    /// }
    ///
    /// struct MyFormatter {
    ///     inner: FixedFractionSample<SampledEmf>,
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
    ///     inner: Emf::all_validations("MyApp".into(), vec![vec![]]).with_sampling().sample_by_fixed_fraction(1.5e-38),
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
    ///     operation: "WillBePotentiallyDropped".to_string(),
    /// }).unwrap();
    ///
    /// // this bypasses sampling
    /// bypass_sampling.store(true, atomic::Ordering::Relaxed);
    /// stream.next(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     operation: "WillRemain".to_string(),
    /// }).unwrap();
    ///
    /// let output = std::str::from_utf8(&output).unwrap();
    /// // Since the probability is 1e-100, we know WillBePotentiallyDropped will be dropped
    /// assert!(!output.contains("WillBePotentiallyDropped"));
    /// assert!(output.contains("WillRemain"));
    /// ```
    ///
    /// [`Format`]: crate::format::Format
    pub fn format_mut(&mut self) -> &mut F {
        &mut self.format
    }
}

impl<F, R> FixedFractionSample<F, R> {
    /// Like [`FixedFractionSample::new`], but also specify the random number generator.
    pub fn with_rng(format: F, rate: f32, rng: R) -> Self {
        assert!(rate.is_finite() && 0.0 < rate && rate <= 1.0);
        Self { format, rate, rng }
    }
}

impl<F: SampledFormat, R: RngCore> Format for FixedFractionSample<F, R> {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        if self.rng.random::<f32>() <= self.rate {
            self.format
                .format_with_sample_rate(entry, output, self.rate)
        } else {
            Ok(())
        }
    }
}
