// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use metrique_writer_core::{EntryWriter, entry::SampleGroupElement};

mod atomics;
mod close_value_impls;
pub mod concat;
mod inflectable_entry_impls;
mod namestyle;

pub use atomics::Counter;
pub use namestyle::NameStyle;

/// Close a given value
///
/// This gives an opportunity do things like stopping timers, collecting fanned-in data, etc.
///
/// If possible, implement this trait for both `&MyValue` and `MyValue`, as this will allow
/// use via smart pointers (e.g. on `Arc<MyValue>`).
///
/// ```
/// use metrique::{CloseValue, CloseValueRef};
///
/// struct MyValue;
///
/// impl CloseValue for &'_ MyValue {
///     type Closed = u32;
///     fn close(self) -> Self::Closed { 42 }
/// }
///
/// impl CloseValue for MyValue {
///     type Closed = u32;
///     fn close(self) -> Self::Closed { self.close_ref() /* delegate to by-ref implementation */ }
/// }
/// ```
///
/// This trait is also used for entries that can be closed as [`CloseEntry`].
///
/// ## Why `CloseValue` is not implemented for `&str` or `&String`
///
/// Since `CloseValue` takes its argument by value, an implementation of `CloseValue`
/// for `&str` or `&String` would have to allocate.
///
/// The most common case this is encountered is when using `#[metrics(subfield)]`:
///
/// ```compile_fail
/// # use std::sync::Arc;
/// use metrique::unit_of_work::metrics;
///
/// #[metrics(subfield)]
/// struct ChildMetrics {
///     field: String,
/// }
///
/// #[metrics]
/// struct MainMetric {
///     // this is allowed when using `#[metrics(subfield)]`
///     #[metrics(flatten)]
///     child: Arc<ChildMetrics>,
/// }
/// ```
///
/// Since `#[metrics(subfield)]` supports taking the fields by reference, as
/// you this would have to allocate and therefore there is no implementation
/// to avoid surprise allocations.
///
/// There are a few options to deal with it:
///
/// 1. Use a `&'static str` or a `#[metrics(value(string))]` enum instead of a `String`, avoiding
///    the allocation.
///     ```rust
///     # use metrique::unit_of_work::metrics;
///     #[metrics(value(string))]
///     enum MyEnum { Foo }
///
///     #[metrics(subfield)]
///     struct ChildMetrics {
///         field_1: &'static str,
///         field_2: MyEnum,
///     }
///     ```
/// 2. Use `#[metrics(subfield_owned)]`, which implements `CloseValue` by value.
///    In that case, you can't use the subfield via an `Arc` but only by-value:
///     ```rust
///     # use metrique::unit_of_work::metrics;
///     #[metrics(subfield_owned)]
///     struct ChildMetrics {
///         field: String,
///     }
///
///     #[metrics]
///     struct MainMetric {
///         // must use by-value
///         #[metrics(flatten)]
///         child: ChildMetrics,
///     }
///     ```
/// 3. If you are fine with allocating, you could make your own string wrapper type.
///     ```rust
///     # use metrique::unit_of_work::metrics;
///     struct StringValue(String);
///     impl metrique::CloseValue for &StringValue {
///         type Closed = String;
///
///         fn close(self) -> String { self.0.clone() }
///     }
///
///     impl metrique::CloseValue for StringValue {
///         type Closed = String;
///
///         fn close(self) -> String { self.0 }
///     }
///
///     #[metrics(subfield)]
///     struct ChildMetrics {
///         field: StringValue,
///     }
///    ```
#[diagnostic::on_unimplemented(
    label = "This type must implement `CloseValue`",
    message = "CloseValue is not implemented for {Self}",
    note = "You may need to add `#[metrics]` to `{Self}` or implement `CloseValue` directly.",
    note = "if {Self} implements `Value` but not `CloseValue`, add `#[metrics(no_close)]`",
    note = "If this type is `&T`, is closed inside a flattened entry, and `T` implements `CloseValue`, consider using `#[metrics(subfield_owned)]`."
)]
pub trait CloseValue {
    /// The type produced by closing this value
    type Closed;

    /// Close the value
    fn close(self) -> Self::Closed;
}

mod private {
    pub trait Sealed {}
}

/// Close a value without taking ownership
///
/// This trait is meant to be used for [`CloseValue`] impls for smart-pointer-like
/// types, as in
///
/// ```
/// use metrique::{CloseValue, CloseValueRef};
///
/// struct Smaht<T>(T);
///
/// impl<T: CloseValueRef> CloseValue for &'_ Smaht<T> {
///     type Closed = T::Closed;
///     fn close(self) -> T::Closed { self.0.close_ref() }
/// }
///
/// impl<T: CloseValueRef> CloseValue for Smaht<T> {
///     type Closed = T::Closed;
///     fn close(self) -> T::Closed { self.0.close_ref() }
/// }
/// ```
///
/// This trait is not to be implemented or called directly. It mostly exists
/// because it makes trait inference a bit smarter (it's also not a full
/// trait alias due to trait inference reasons).
#[diagnostic::on_unimplemented(
    message = "CloseValueRef is not implemented for {Self}",
    note = "You may need to add `#[metrics]` to `{Self}` or implement `CloseValueRef` directly."
)]
pub trait CloseValueRef: private::Sealed {
    /// The type produced by closing this value
    type Closed;

    /// Close the value
    fn close_ref(&self) -> Self::Closed;
}

impl<C, T> private::Sealed for T where for<'a> &'a Self: CloseValue<Closed = C> {}

impl<C, T> CloseValueRef for T
where
    for<'a> &'a Self: CloseValue<Closed = C>,
{
    type Closed = C;
    fn close_ref(&self) -> Self::Closed {
        <&Self>::close(self)
    }
}

/// An object that can be closed into an [InflectableEntry]. This is the
/// normal way of generating a metric entry - by starting with a a struct
/// that implements this trait (that is generally generated using the `#[metrics]` macro),
/// wrapping it in a [`RootEntry`] to generate an [`Entry`], and then emitting that
/// to an [`EntrySink`].
///
/// This is just a trait alias for `CloseValue<Closed: InflectableEntry>`.
///
/// [close-value]: CloseValue
/// [`Entry`]: metrique_writer_core::Entry
/// [`EntrySink`]: metrique_writer_core::EntrySink
/// [`RootEntry`]: https://docs.rs/metrique/0.1/metrique/struct.RootEntry.html
pub trait CloseEntry: CloseValue<Closed: InflectableEntry> {}
impl<T: ?Sized + CloseValue<Closed: InflectableEntry>> CloseEntry for T {}

/// A trait for metric entries where the names of the fields can be "inflected"
/// using a [`NameStyle`]. This defines the interface for metric *sources*
/// that want to be able to generate metric structs that can be renamed
/// without having any string operations happen at runtime.
///
/// Both `InflectableEntry` and [`Entry`] are intended to be "pure" structs - all
/// references to channels, counters and the like are expected to be resolved when
/// creating the `InflectableEntry`.
///
/// An `InflectableEntry` with any specific set of type parameters is equivalent to an
/// [`Entry`]. It should be wrapped by a wrapper that implements [`Entry`] and delegates
/// to it with a particular set of type parameters, for example `RootEntry`, and then
/// emitting that to an [`EntrySink`].
///
/// The normal way of generating a metric entry is by starting with a struct
/// that implements [`CloseEntry`] (that is generally generated
/// using the `#[metrics]` macro), wrapping it in a `RootEntry` to generate an
/// [`Entry`], and then emitting that to an entry sink.
///
/// Design note: in theory you could have a world where `InflectableEntry`
/// and [`Entry`] are the same trait (where the sinks use the default type parameters).
/// In practice, it is desired that the trait [`Entry`] will have very few breaking
/// changes since it needs to be identical throughout a program that wants to emit
/// metrics to a single destination, and therefore `InflectableEntry` is kept separate.
///
/// ## Manual Implementations
///
/// Currently, there is no (stable) non-macro way of generating an [`InflectableEntry`]
/// that actually inflects names. If you want to make a manual entry, it is recommended
/// to implement the [`Entry`] trait, then use a field with `#[metrics(flatten_entry)]`
/// as follows - though note that this will ignore inflections:
///
/// ```
/// use metrique::unit_of_work::metrics;
/// use metrique::writer::{Entry, EntryWriter};
///
/// struct MyCustomEntry;
///
/// impl Entry for MyCustomEntry {
///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
///         writer.value("custom", "custom");
///     }
/// }
///
/// #[metrics]
/// struct MyMetric {
///     #[metrics(flatten_entry, no_close)]
///     field: MyCustomEntry,
/// }
/// ```
///
/// [`Entry`]: metrique_writer_core::Entry
/// [`NameStyle`]: namestyle::NameStyle
/// [`Entry`]: metrique_writer_core::Entry
/// [`EntrySink`]: metrique_writer_core::EntrySink
pub trait InflectableEntry<NS: namestyle::NameStyle = namestyle::Identity> {
    /// Write this metric entry to an EntryWriter
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>);
    /// Sample group
    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        vec![].into_iter()
    }
}
