// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::entry::SampleGroupElement;

use super::{Entry, EntryWriter};

/// Merges 2 [Entry] objecs by value. See [Entry::merge].
#[derive(Clone, Debug)]
pub struct Merged<E1, E2>(pub(super) E1, pub(super) E2);

impl<E1: Entry, E2: Entry> Entry for Merged<E1, E2> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        self.0.write(writer);
        self.1.write(writer);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        self.0.sample_group().chain(self.1.sample_group())
    }
}


/// Merges 2 [Entry] objects by reference. See [Entry::merge_by_ref].
#[derive(Debug)]
pub struct MergedRef<'a, E1: ?Sized, E2: ?Sized>(pub(super) &'a E1, pub(super) &'a E2);

impl<E1: Entry + ?Sized, E2: Entry + ?Sized> Entry for MergedRef<'_, E1, E2> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        self.0.write(writer);
        self.1.write(writer);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        self.0.sample_group().chain(self.1.sample_group())
    }
}

impl<E1: ?Sized, E2: ?Sized> Clone for MergedRef<'_, E1, E2> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E1: ?Sized, E2: ?Sized> Copy for MergedRef<'_, E1, E2> {}
