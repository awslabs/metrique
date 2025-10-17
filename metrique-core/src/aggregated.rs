// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Aggregated field support for embedding keyless aggregation within regular metrics.

use crate::CloseValue;
use metrique_writer_core::{
    merge::{AggregatableEntry, AggregatedEntry},
    Entry, EntryWriter,
};
use std::borrow::Cow;

/// A wrapper type for aggregated fields within regular metrics.
/// 
/// Only supports keyless aggregation (`Key = ()`), allowing all entries to be merged
/// into a single aggregated result that can be flattened into the parent metric.
pub struct Aggregated<T: AggregatableEntry<Key = ()>> {
    aggregated: Option<T::Aggregated>,
}

impl<T: AggregatableEntry<Key = ()>> Aggregated<T> {
    /// Create a new empty aggregated field.
    pub fn new() -> Self {
        Self { aggregated: None }
    }
    
    /// Add an entry to the aggregation.
    pub fn add(&mut self, entry: T) {
        match &mut self.aggregated {
            Some(agg) => agg.aggregate_into(&entry),
            None => {
                let mut agg = T::new_aggregated(());
                agg.aggregate_into(&entry);
                self.aggregated = Some(agg);
            }
        }
    }
    
    /// Get the current aggregated entry, if any.
    pub fn get(&self) -> Option<&T::Aggregated> {
        self.aggregated.as_ref()
    }
}

impl<T: AggregatableEntry<Key = ()>> Default for Aggregated<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper for optional aggregated entries to support flattening.
pub struct AggregatedClosed<T>(Option<T>);

impl<T: AggregatableEntry<Key = ()>> CloseValue for Aggregated<T> 
where
    T::Aggregated: Entry,
{
    type Closed = AggregatedClosed<T::Aggregated>;
    
    fn close(self) -> Self::Closed {
        AggregatedClosed(self.aggregated)
    }
}

// Entry implementation for our wrapper type
impl<T: Entry> Entry for AggregatedClosed<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        if let Some(entry) = &self.0 {
            entry.write(writer);  // Flatten directly into parent
        }
    }
    
    fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
        std::iter::empty()  // Use parent's sample group
    }
}

// InflectableEntry implementation for macro compatibility
impl<T: Entry, NS: crate::NameStyle> crate::InflectableEntry<NS> for AggregatedClosed<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        Entry::write(self, writer)
    }
    
    fn sample_group(&self) -> impl Iterator<Item = (Cow<'static, str>, Cow<'static, str>)> {
        Entry::sample_group(self)
    }
}
