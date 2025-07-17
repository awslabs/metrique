// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Defines the [`SampledFormat`] trait, which allows for formats that can be sampled.

use std::io;

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
