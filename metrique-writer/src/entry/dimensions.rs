// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::CowStr;
use metrique_writer_core::{
    Entry, EntryConfig, EntryWriter, MetricFlags, Observation, Unit, ValidationError, Value,
    ValueWriter,
};
use smallvec::SmallVec;
use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
    {borrow::Cow, time::SystemTime},
};

/// Adds a set of global dimensions to every metric of an entry except for those included in the
/// `global_dimensions_denylist` as (class, instance) pairs.
///
/// [`WithDimensions`] adds a set of dimensions to *one* metric value of an entry while `WithGlobalDimensions` adds a
/// set of dimensions to *every* metric value of an entry.
///
/// The const `N` defines how many of the pairs will be stored inline with the entry before being spilled to the heap.
/// In most cases, the number of dimensions is known and setting `N` accordingly will avoid an allocation.
///
/// [`WithDimensions`]: metrique_writer_core::value::WithDimensions
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WithGlobalDimensions<E, const N: usize> {
    entry: E,
    global_dimensions: SmallVec<[(CowStr, CowStr); N]>,
    global_dimensions_denylist: HashSet<CowStr>,
}

impl<E, const N: usize> Deref for WithGlobalDimensions<E, N> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl<E, const N: usize> DerefMut for WithGlobalDimensions<E, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entry
    }
}

impl<E, const N: usize> From<E> for WithGlobalDimensions<E, N> {
    fn from(entry: E) -> Self {
        Self {
            entry,
            global_dimensions: Default::default(),
            global_dimensions_denylist: Default::default(),
        }
    }
}

impl<E, const N: usize> WithGlobalDimensions<E, N> {
    pub(crate) fn new(
        entry: E,
        global_dimensions: SmallVec<[(CowStr, CowStr); N]>,
        global_dimensions_denylist: HashSet<CowStr>,
    ) -> Self {
        WithGlobalDimensions {
            entry,
            global_dimensions,
            global_dimensions_denylist,
        }
    }
}

impl<E, const N: usize> WithGlobalDimensions<E, N> {
    /// Add all of the given global dimensions to `entry`.
    ///
    /// Note that `N` should be chosen to match the upper bound length of `dimensions`.
    pub fn new_with_global_dimensions<C, I>(
        entry: E,
        global_dimensions: impl IntoIterator<Item = (C, I)>,
        global_dimensions_denylist: HashSet<CowStr>,
    ) -> Self
    where
        C: Into<CowStr>,
        I: Into<CowStr>,
    {
        Self {
            entry,
            global_dimensions: global_dimensions
                .into_iter()
                .map(|(c, i)| (c.into(), i.into()))
                .collect(),
            global_dimensions_denylist,
        }
    }

    /// Return the current global dimensions.
    pub fn global_dimensions(&self) -> &[(CowStr, CowStr)] {
        &self.global_dimensions
    }

    /// Return the current global dimensions denylist.
    pub fn global_dimensions_denylist(&self) -> &HashSet<CowStr> {
        &self.global_dimensions_denylist
    }

    /// Add a new (`class`, `instance`) global dimension to the current global dimensions.
    pub fn add_global_dimension(
        &mut self,
        class: impl Into<CowStr>,
        instance: impl Into<CowStr>,
    ) -> &mut Self {
        self.global_dimensions.push((class.into(), instance.into()));
        self
    }

    /// Clear the current global dimensions.
    pub fn clear_global_dimensions(&mut self) {
        self.global_dimensions.clear()
    }

    /// Clear the current global dimensions denylist.
    pub fn clear_global_dimensions_denylist(&mut self) {
        self.global_dimensions_denylist.clear()
    }
}

impl<E: Entry, const N: usize> Entry for WithGlobalDimensions<E, N> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        struct EntryWriterWrapper<'a, W> {
            writer: W,
            global_dimensions: &'a [(CowStr, CowStr)],
            global_dimensions_denylist: &'a HashSet<CowStr>,
        }

        impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for EntryWriterWrapper<'_, W> {
            fn timestamp(&mut self, timestamp: SystemTime) {
                self.writer.timestamp(timestamp);
            }

            fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
                let name: Cow<'a, str> = name.into();
                // Don't wrap the value with global_dimensions if it's in the global dimensions denylist
                if self.global_dimensions_denylist.contains(&name) {
                    self.writer.value(name, value)
                } else {
                    self.writer.value(
                        name,
                        &ValueWrapper {
                            value,
                            global_dimensions: self.global_dimensions,
                        },
                    )
                }
            }

            fn config(&mut self, config: &'a dyn EntryConfig) {
                self.writer.config(config);
            }
        }

        self.entry.write(&mut EntryWriterWrapper {
            writer,
            global_dimensions: self.global_dimensions(),
            global_dimensions_denylist: self.global_dimensions_denylist(),
        })
    }
}

struct ValueWrapper<'a, V> {
    value: V,
    global_dimensions: &'a [(CowStr, CowStr)],
}

impl<V: Value> Value for ValueWrapper<'_, V> {
    fn write(&self, writer: impl ValueWriter) {
        struct ValueWriterWrapper<'a, W> {
            writer: W,
            global_dimensions: &'a [(CowStr, CowStr)],
        }

        impl<W: ValueWriter> ValueWriter for ValueWriterWrapper<'_, W> {
            fn string(self, value: &str) {
                self.writer.string(value)
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                global_dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                flags: MetricFlags<'_>,
            ) {
                #[allow(clippy::map_identity)]
                // https://github.com/rust-lang/rust-clippy/issues/9280
                self.writer.metric(
                    distribution,
                    unit,
                    global_dimensions
                        .into_iter()
                        .map(|(k, v)| (k, v)) // reborrow to align lifetimes
                        .chain(self.global_dimensions.iter().map(|(c, i)| (&**c, &**i))),
                    flags,
                )
            }

            fn error(self, error: ValidationError) {
                self.writer.error(error)
            }
        }

        self.value.write(ValueWriterWrapper {
            writer,
            global_dimensions: self.global_dimensions,
        })
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashSet,
        ops::{Deref, DerefMut},
    };

    use metrique_writer_core::{Entry, EntryWriter};
    use smallvec::SmallVec;

    use crate::{CowStr, entry::WithGlobalDimensions};

    #[test]
    fn test_deref_and_deref_mut_for_global_dimensions() {
        #[derive(Debug, PartialEq, Eq)]
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {
                panic!("Not to be called in this test!");
            }
        }

        let mut global_dimensions: SmallVec<[(CowStr, CowStr); 1]> = SmallVec::with_capacity(1);
        global_dimensions.push(("az".into(), "us-east-1a".into()));
        let global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
        let mut global_dimensions_entry =
            WithGlobalDimensions::new(TestEntry, global_dimensions, global_dimensions_denylist);

        assert_eq!(global_dimensions_entry.deref(), &TestEntry);
        assert_eq!(global_dimensions_entry.deref_mut(), &TestEntry);
    }

    #[test]
    fn test_global_dimensions() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {
                panic!("Not to be called in this test!");
            }
        }

        let mut global_dimensions: SmallVec<[(CowStr, CowStr); 1]> = SmallVec::with_capacity(1);
        global_dimensions.push(("az".into(), "us-east-1a".into()));
        let global_dimensions_denylist: HashSet<CowStr> = HashSet::new();
        let mut global_dimensions_entry =
            WithGlobalDimensions::new(TestEntry, global_dimensions, global_dimensions_denylist);
        global_dimensions_entry.add_global_dimension("cell", "cell1");

        let mut converted_global_dimensions: Vec<(&str, &str)> = Vec::new();
        for (c, i) in global_dimensions_entry.global_dimensions() {
            converted_global_dimensions.push((&c, &i));
        }

        assert_eq!(
            converted_global_dimensions,
            &[("az", "us-east-1a"), ("cell", "cell1")]
        );

        global_dimensions_entry.clear_global_dimensions();
        assert_eq!(global_dimensions_entry.global_dimensions(), &[]);
    }
}
