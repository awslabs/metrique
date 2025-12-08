// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Defines the [`SampledFormat`] trait, which allows for formats that can be sampled.

use std::{borrow::Cow, io};

use crate::{Entry, IoStreamError, format::Format};

/// Allows for sampleable formats, with a "sample rate" that will automatically compensate for entries that
/// were sampled by that fraction. This allow services to trade a lower-accuracy metric for reduced time emitting and
/// processing metrics.
pub trait SampledFormat: Format {
    /// Like [`Format::format()`], but also associate the entry with a sample rate in the range `(0, 1]`.
    ///
    /// A sample rate of 0.1 indicates that this was the only entry out of ten similar entries that was actually
    /// formatted. The metrics consumer should extrapolate the other nine entries.
    fn format_with_sample_rate(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
        rate: f32,
    ) -> Result<(), IoStreamError>;
}

/// A type that can be converted to a sample group
///
/// Sample groups are used by [congress sampling] to ensure that logs for rare conditions are
/// still sampled, even if the overall sample rate is low, by ensuring that operations from
/// every value of sample groups is sampled.
///
/// For example, when writing metrics for an API server, it is common to mark the operation (route)
/// and status code as sample groups, to ensure every (operation, status code) gets a metric.
///
/// [congress sampling]: https://docs.rs/metrique-writer/0.1/metrique_writer/sample/struct.CongressSample.html
pub trait SampleGroup {
    /// Return the value as a sample group
    fn as_sample_group(&self) -> Cow<'static, str>;
}

impl SampleGroup for &'static str {
    fn as_sample_group(&self) -> Cow<'static, str> {
        Cow::Borrowed(self)
    }
}
