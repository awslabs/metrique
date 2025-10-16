// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Merging support for aggregating multiple entries into a single entry.

use crate::Entry;

pub mod sink;

pub use sink::{MergeConfig, MergingEntrySink};

/// An entry that can be merged with other entries of the same type.
///
/// This enables in-memory aggregation of metrics before emission, reducing
/// the number of metric records sent to the backend.
pub trait MergeableEntry: Entry {
    /// The type that accumulates merged entries.
    type Merged: MergedEntry<Source = Self>;

    /// Create a new merger for this entry type.
    fn new_merged() -> Self::Merged;
}

/// Accumulates multiple entries and produces a merged result.
pub trait MergedEntry: Entry {
    /// The source entry type being merged.
    type Source: MergeableEntry<Merged = Self>;

    /// Merge another entry into this accumulator.
    fn merge_into(&mut self, entry: &Self::Source);

    /// Get the number of entries merged so far.
    fn count(&self) -> usize;
}

/// Strategy for merging numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Sum all values (for counters).
    Sum,
    /// Keep the last value (for gauges).
    Last,
    /// Keep the minimum value.
    Min,
    /// Keep the maximum value.
    Max,
    /// Calculate the average.
    Average,
}
