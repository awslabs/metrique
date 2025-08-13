// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module contains the [Entry] trait, which represents an entry that can be written to
//! an [EntryWriter] in order to emit metrics

use std::{any::Any, borrow::Cow, sync::Arc, time::SystemTime};

mod boxed;
pub use boxed::BoxEntry;

mod map;

mod merged;
pub use merged::{Merged, MergedRef};

use crate::Value;

/// The core trait to be implemented by application data structures holding metric values.
///
/// Implementations of [`Entry`] should generally be pure functions, emitting the same metric values
/// independent of the environment. External dependencies, such as metrics that are relative to
/// the current time, or metrics that refer to the value behind a mutex or atomic, should be resolved
/// before creating an [`Entry`].
///
/// It's analogous to the [`std::fmt::Display`] trait that works with a [`std::fmt::Formatter`] to display a value as a
/// string. In this case, the implementer of `Entry` contains the metric data and works with an [`EntryWriter`] to write
/// the data as a metric entry in some format. The `EntryWriter` trait abstracts away the different formatting logic
/// from the `Entry` implementer, just like `Formatter` does for `Display` implementers.
///
/// ## Creating Entries
///
/// The `metrique` crate provides abstractions for easily creating an [`Entry`] while having a step
/// where dependencies of the metric on external value can be resolved.
///
/// If your [`Entry`] does not need to contain external dependencies and you are not using `metrique`,
/// the easiest way of creating an [`Entry`] is by using `derive(Entry)`, for example:
///
/// ```
/// # use std::time::{Duration, SystemTime};
/// # use metrique_writer::Entry;
/// # use metrique_writer::unit::{AsBytes, AsMicroseconds};
///
/// #[derive(Entry, Debug)]
/// #[entry(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     #[entry(timestamp)]
///     request_start: SystemTime,
///     // emitted as `0` or `1`
///     success: bool,
///     // will be skipped if `None`
///     optional_value: Option<u32>,
///     byte_size: AsBytes<u64>,
///     // An `Entry` can nest inner `Entry` structs
///     #[entry(flatten)]
///     subprocess: SubprocessMetrics,
/// }
///
/// #[derive(Entry, Default, Debug)]
/// #[entry(rename_all = "PascalCase")]
/// struct SubprocessMetrics {
///     counter: u32,
///     // this Duration will be emitted as milliseconds (the default)
///     duration: Duration,
///     // this Duration will be emitted as microseconds
///     duration_microseconds: AsMicroseconds<Duration>,
/// }
/// ```
///
/// You can also implement the [`Entry`] trait manually (this does the same as the
/// macro-using code above):
///
/// ```no_run
/// # use metrique_writer_core::{Entry, EntryWriter};
/// # use std::time::{Duration, SystemTime};
/// # use metrique_writer::unit::{AsBytes, AsMicroseconds};
///
/// struct RequestMetrics {
///     request_start: SystemTime,
///     // emitted as `0` or `1`
///     success: bool,
///     // will be skipped if `None`
///     optional_value: Option<u32>,
///     byte_size: AsBytes<u64>,
///     // Multiple entries can be merged or flattened into a single written entry by
///     // invoking [`Entry::write()`] multiple times with the same `EntryWriter`.
///     // This makes nesting separate structs easy.
///     subprocess: SubprocessMetrics,
/// }
///
/// struct SubprocessMetrics {
///     counter: u32,
///     // this Duration will be emitted as milliseconds (the default
///     // of `impl Value for Duration` and `impl MetricValue for Duration`).
///     duration: Duration,
///     // this Duration will be emitted as microseconds
///     duration_microseconds: AsMicroseconds<Duration>,
/// }
///
/// impl Entry for RequestMetrics {
///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
///         writer.timestamp(self.request_start);
///         writer.value("Success", &self.success);
///         writer.value("OptionalValue", &self.optional_value);
///         writer.value("ByteSize", &self.byte_size);
///         self.subprocess.write(writer);
///     }
/// }
///
/// impl Entry for SubprocessMetrics {
///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
///         writer.value("Counter", &self.counter);
///         writer.value("Duration", &self.duration);
///         writer.value("DurationMicroseconds", &self.duration_microseconds);
///     }
/// }
/// ```
///
pub trait Entry {
    /// Write the metric values contained in this entry to the format-provided [`EntryWriter`]. The `writer` corresponds
    /// to an atomic entry written to the metrics consumer, like CloudWatch.
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>);

    /// The key used to group "similar" entries when sampling. Defaults to the empty group.
    ///
    /// If the output format is unsampled or is using a naive sampling strategy, like a
    /// [`FixedFractionSample`], this is unused.
    ///
    /// For adaptive sampling strategies, like [`CongressSample`], the sample group should reflect
    /// representative buckets for the service. A sane starting point for request-reply services would include the API
    /// name and resulting status code. This ensures that less frequent APIs and less frequent status codes aren't lost
    /// in a low sample rate.
    ///
    /// The order of (key, value) pairs in the group doesn't matter, but each key must be unique. Implementations should
    /// panic on duplicate keys in debug builds but only emit a [`tracing`] error otherwise.
    ///
    /// # Example
    /// For a request-reply service, typically the API name and result should be used.
    /// ```
    /// # use metrique_writer::Entry;
    /// # use std::collections::HashMap;
    /// #[derive(Entry, Default, Debug)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct RequestMetrics {
    ///     #[entry(sample_group)]
    ///     operation: &'static str,
    ///     #[entry(sample_group)]
    ///     result: &'static str,
    ///     some_counter: u64,
    ///     // ...
    /// }
    ///
    /// let metrics = RequestMetrics {
    ///     operation: "Foo",
    ///     result: "ValidationError",
    ///     ..Default::default()
    /// };
    /// let sample_group = metrics.sample_group().collect::<HashMap<_, _>>();
    /// assert_eq!(&sample_group["Operation"], "Foo");
    /// assert_eq!(&sample_group["Result"], "ValidationError");
    /// ```
    ///
    /// [`FixedFractionSample`]: https://docs.rs/metrique-writer/0.1/metrique_writer/sample/struct.FixedFractionSample.html
    /// [`CongressSample`]: https://docs.rs/metrique-writer/0.1/metrique_writer/sample/struct.CongressSample.html
    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        [].into_iter()
    }

    /// Create a new entry that writes all the contents of this entry and then all of the contents of `other`.
    ///
    /// Useful to merge in global constants or metrics collected by different subsystems.
    fn merge<E>(self, other: E) -> Merged<Self, E>
    where
        Self: Sized,
    {
        Merged(self, other)
    }

    /// Like [`Entry::merge`], but does so by reference.
    fn merge_by_ref<'a, E: 'a + Entry>(&'a self, other: &'a E) -> MergedRef<'a, Self, E> {
        MergedRef(self, other)
    }

    /// Move the entry to the heap and rely on dynamic dispatch.
    ///
    /// Useful for creating heterogeneous collections of entries.
    fn boxed(self) -> BoxEntry
    where
        Self: Sized + Send + 'static,
    {
        BoxEntry::new(self)
    }
}

/// A `(key, value)` pair, part of a sample group
pub type SampleGroupElement = (Cow<'static, str>, Cow<'static, str>);

/// [`Entry`] that will write no fields.
///
/// Useful for specifying empty globals when attaching [`crate::global`] sinks.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EmptyEntry;

impl Entry for EmptyEntry {
    fn write<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {}
}

/// Trait for format-specific Entry configuration, formats will downcast this to the specific config
pub trait EntryConfig: Any + std::fmt::Debug {}

/// Provided by a format for each atomic entry that will be written to the metric destination.
///
/// Note that the lifetime `'a` corresponds to the lifetime of all metric names (if [`Cow::Borrowed`]) for the format
/// entry. In most cases, metric names are `'static` strings anyways, but this allows formats to store hash sets of all
/// written names for validation without allocating copies.
pub trait EntryWriter<'a> {
    /// Set the timestamp associated with the metric entry. If this is never invoked, formats are free to use the
    /// current system time.
    ///
    /// This must never panic, but if invoked twice may result in a validation panic on [`crate::EntrySink::append()`]
    /// for test sinks or a `tracing` event on production queues.
    fn timestamp(&mut self, timestamp: SystemTime);

    /// Record a metric [`Value`] in the entry. Each format may have more specific requirements, but typically each
    /// `name` must be unique within the entry.
    ///
    /// This must never panic, but if invalid names or values may result in a panic on [`crate::EntrySink::append()`]
    /// for test sinks or a `tracing` event on production queues.
    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized));

    /// Pass format-specific entry configuration. Formatters should ignore configuration they are unaware of.
    fn config(&mut self, config: &'a dyn EntryConfig);
}

impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for &mut W {
    fn timestamp(&mut self, timestamp: SystemTime) {
        (**self).timestamp(timestamp)
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        (**self).value(name, value)
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        (**self).config(config)
    }
}

impl<T: Entry + ?Sized> Entry for &T {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}

impl<T: Entry> Entry for Option<T> {
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

impl<T: Entry + ?Sized> Entry for Box<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}

impl<T: Entry + ?Sized> Entry for Arc<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}

impl<T: Entry + ToOwned + ?Sized> Entry for Cow<'_, T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write(writer)
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        (**self).sample_group()
    }
}
