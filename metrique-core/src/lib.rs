// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]

//! This crate contains the traits for the `metrique` set of libraries. Generally, you
//! should not depend on this crate directly. Instead, use `metrique`

use metrique_writer_core::{EntryWriter, entry::SampleGroupElement};

mod atomics;
mod close_value_impls;
mod inflectable_entry_impls;
mod namestyle;

pub use atomics::Counter;
pub use namestyle::NameStyle;

/// Close a given value
///
/// This gives an opportunity do things like stopping timers, collecting fanned-in data, etc.
///
/// The normal way of generating a metric entry is by starting with a struct
/// that implements `CloseValue<Closed: MetricEntry>` (that is generally generated
/// using the `#[metrics]` macro), wrapping it in a `RootEntry` to generate an
/// [`Entry`], and then emitting that to an [`EntrySink`].
///
/// [`Entry`]: metrique_writer::Entry
/// [`EntrySink`]: metrique_writer::EntrySink
#[diagnostic::on_unimplemented(
    message = "CloseValue is not implemented for {Self}",
    note = "You may need to add `#[metrics]` to `{Self}` or implement `CloseValue` directly."
)]
pub trait CloseValue {
    /// The type produced by closing this value
    type Closed;

    /// Close the value
    fn close(self) -> Self::Closed;
}

/// A trait for metric entries where the names of the fields can be "inflected"
/// using a [`NameStyle`]. This defines the interface for metric *sources*
/// that want to be able to generate metric structs that can be renamed
/// without having any string operations happen at runtime.
///
/// Both `MetricEntry` and [`Entry`] are intended to be "pure" structs - all
/// references to channels, counters and the like are expected to be resolved when
/// creating the `MetricEntry`.
///
/// An `InflectableEntry` with any specific set of type parameters is equivalent to an
/// [`Entry`]. It should be wrapped by a wrapper that implements [`Entry`] and delegates
/// to it with a particular set of type parameters, for example `RootEntry`, and then
/// emitting that to an [`EntrySink`].
///
/// The normal way of generating a metric entry is by starting with a struct
/// that implements [`CloseValue<Closed: MetricEntry>`][close-value] (that is generally generated
/// using the `#[metrics]` macro), wrapping it in a `RootEntry` to generate an
/// [`Entry`], and then emitting that to an entry sink.
///
/// Design note: in theory you could have a world where `InflectableEntry`
/// and [`Entry`] are the same trait (where the sinks use the default type parameters).
/// In practice, it is desired that the trait [`Entry`] will have very few breaking
/// changes since it needs to be identical throughout a program that wants to emit
/// metrics to a single destination, and therefore `InflectableEntry` is kept separate.
///
/// [`Entry`]: metrique_writer::Entry
/// [`NameStyle`]: namestyle::NameStyle
/// [`Entry`]: metrique_writer::Entry
/// [`EntrySink`]: metrique_writer::EntrySink
/// [close-value]: CloseValue
pub trait InflectableEntry<NS: namestyle::NameStyle = namestyle::Identity> {
    /// Write this metric entry to an EntryWriter
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>);
    /// Sample group
    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        vec![].into_iter()
    }
}

/// Close a value without taking ownership
///
/// This trait is not to be *called directly*, and it will also not be called
/// directly by the `#[metric]` macro. It is instead used by the following blanket impls:
/// 1. impl `CloseValue` for `T where T: CloseValueRef`
/// 2. impl `CloseValue` for `Smart<T> where T: CloseValueRef` for various
///    smart pointer types.
#[diagnostic::on_unimplemented(
    message = "CloseValueRef is not implemented for {Self}",
    note = "You may need to add `#[metrics]` to `{Self}` or implement `CloseValueRef` directly."
)]
pub trait CloseValueRef {
    /// The type produced by closing this value
    type Closed;
    /// Close the value
    fn close_ref(&self) -> Self::Closed;
}

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef> CloseValue for T {
    type Closed = <Self as CloseValueRef>::Closed;

    /// Close the value
    fn close(self) -> Self::Closed {
        self.close_ref()
    }
}
