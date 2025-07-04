// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains configurations that are mostly useful for EMF-format metrics
//!
//! The configurations are in this crate in the interest of interoperability

use std::{borrow::Cow, slice};

use crate::EntryConfig;

/// This config enables splitting entries in case of multiple dimension values.
///
/// Mostly useful for EMF, but your own custom formatters can use it too.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AllowSplitEntries(());

impl AllowSplitEntries {
    /// Create a new [AllowSplitEntries]
    pub const fn new() -> Self {
        Self(())
    }
}

impl EntryConfig for AllowSplitEntries {}

/// This struct is mostly useful for the EMF internal implementation
pub struct DimensionsIterator<'a> {
    inner: slice::Iter<'a, Cow<'static, str>>,
}

impl<'a> Iterator for DimensionsIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|d| &**d)
    }
}

#[derive(Clone, Debug)]
pub struct EntryDimensions {
    dimensions: Cow<'static, [Cow<'static, [Cow<'static, str>]>]>,
}

impl EntryDimensions {
    /// Create a new [EntryDimensions]
    pub const fn new(dimensions: Cow<'static, [Cow<'static, [Cow<'static, str>]>]>) -> Self {
        Self { dimensions }
    }

    /// A wrapper around [`EntryDimensions::new`] that avoids one layer of Cow.
    pub const fn new_static(dimensions: &'static [Cow<'static, [Cow<'static, str>]>]) -> Self {
        Self {
            dimensions: Cow::Borrowed(dimensions),
        }
    }

    /// This method is mostly useful for the EMF internal implementation
    pub fn is_empty(&self) -> bool {
        self.dimensions.is_empty()
    }

    /// This method is mostly useful for the EMF internal implementation
    ///
    /// Intentionally abstracts over the EntryDimensions implementation details to reduce breakage risk
    pub fn dim_sets<'a>(&'a self) -> impl Iterator<Item = DimensionsIterator<'a>> {
        self.dimensions.iter().map(|inner| DimensionsIterator {
            inner: inner.iter(),
        })
    }
}

impl EntryConfig for EntryDimensions {}

#[cfg(test)]
mod test {
    use super::EntryDimensions;
    use std::borrow::Cow;

    #[test]
    fn test_misc_coverage() {
        // coverage doesn't count const fns. Get it to avoid annoying me.
        assert!(
            EntryDimensions::new(Cow::Borrowed(&[Cow::Borrowed(&[])])).dimensions[0].is_empty()
        );
        assert!(EntryDimensions::new_static(&[Cow::Borrowed(&[])]).dimensions[0].is_empty());
    }
}
