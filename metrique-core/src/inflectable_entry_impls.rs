// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

// Delegate Entry impls for references and standard containers

use std::{borrow::Cow, sync::Arc};

use metrique_writer_core::{EntryWriter, entry::SampleGroupElement};

use crate::{InflectableEntry, namestyle::NameStyle};

impl<NS: NameStyle, T: InflectableEntry<NS>> InflectableEntry<NS> for &T {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}

impl<NS: NameStyle, T: InflectableEntry<NS>> InflectableEntry<NS> for Option<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        if let Some(entry) = self.as_ref() {
            entry.write(writer)
        }
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        if let Some(entry) = self.as_ref() {
            itertools::Either::Left(entry.sample_group())
        } else {
            itertools::Either::Right([].into_iter())
        }
    }
}

impl<NS: NameStyle, T: InflectableEntry<NS> + ?Sized> InflectableEntry<NS> for Box<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}

impl<NS: NameStyle, T: InflectableEntry<NS> + ?Sized> InflectableEntry<NS> for Arc<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}

impl<NS: NameStyle, T: InflectableEntry<NS> + ToOwned + ?Sized> InflectableEntry<NS>
    for Cow<'_, T>
{
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}
