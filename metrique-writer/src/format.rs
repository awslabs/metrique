// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains various utilities for [`Format`]

use std::{collections::HashSet, io};

use metrique_writer_core::{
    Entry,
    stream::{EntryIoStream, IoStreamError},
};

pub use metrique_writer_core::format::Format;
use smallvec::SmallVec;

use crate::{
    CowStr,
    entry::WithGlobalDimensions,
    stream::{MergeGlobalDimensions, MergeGlobals},
};

/// Extension trait for [`Format`]. This adds methods that use types not
/// present within [`metrique_writer_core`].
pub trait FormatExt: Format {
    /// Bind the format to an `output` IO destination to create an [`EntryIoStream`].
    ///
    /// This is the way to get an [`EntryIoStream`] from a [`Format`]
    /// and an [`impl std::io::Write`][std::io::Write].
    ///
    /// ## Example
    ///
    /// This example sets up a global entry sink named `ServiceMetrics` that outputs to stdout
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    Entry,
    /// #    GlobalEntrySink,
    /// #    sink::{AttachGlobalEntrySinkExt, global_entry_sink},
    /// #    format::{FormatExt as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # let log_dir = tempfile::tempdir().unwrap();
    ///
    /// global_entry_sink! { ServiceMetrics }
    ///
    /// let _join = ServiceMetrics::attach_to_stream(Emf::all_validations("MyApp".into(), vec![vec![]])
    ///     .output_to(std::io::stdout()));
    ///
    /// // and then, for example:
    /// #[derive(Entry, Default)]
    /// struct MyMetrics {
    ///  field: usize
    /// }
    ///
    /// let metric_base = MyMetrics { field: 0 };
    /// let mut metric = ServiceMetrics::append_on_drop(metric_base);
    /// ```
    fn output_to<O>(self, output: O) -> FormattedEntryIoStream<Self, O>
    where
        Self: Sized,
    {
        FormattedEntryIoStream {
            format: self,
            output,
        }
    }

    /// Bind the format to a tracing-subscriber 0.3 `output` IO destination to create an [`EntryIoStream`].
    ///
    /// This does not use tracing-subscriber's Metadata feature.
    ///
    /// If tracing-subscriber releases a different version of the `MakeWriter` trait, this function
    /// will keep supporting the 0.3 version of the trait, and a different function will be added
    /// that supports the newer version of the trait.
    ///
    /// There are several nice tracing-subscriber writers that implement MakeWriter, including
    /// tracing-appender's rolling writer.
    ///
    /// ## Example
    ///
    /// This example sets up a global entry sink named `ServiceMetrics` that outputs to
    /// a rotating file
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    Entry,
    /// #    GlobalEntrySink,
    /// #    sink::{AttachGlobalEntrySinkExt, global_entry_sink},
    /// #    format::{FormatExt as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # let log_dir = tempfile::tempdir().unwrap();
    /// use tracing_appender::rolling::{RollingFileAppender, Rotation};
    /// global_entry_sink! { ServiceMetrics }
    ///
    /// let _join = ServiceMetrics::attach_to_stream(Emf::all_validations("MyApp".into(), vec![vec![]])
    ///     .output_to_makewriter(
    ///          RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
    ///     )
    /// );
    ///
    /// // and then, for example:
    /// #[derive(Entry, Default)]
    /// struct MyMetrics {
    ///  field: usize
    /// }
    ///
    /// let metric_base = MyMetrics { field: 0 };
    /// let mut metric = ServiceMetrics::append_on_drop(metric_base);
    /// ```
    #[cfg(feature = "tracing_subscriber_03")]
    fn output_to_makewriter<O>(self, output: O) -> FormattedMakeWriterEntryIoStream<Self, O>
    where
        Self: Sized,
    {
        FormattedMakeWriterEntryIoStream {
            format: self,
            output,
        }
    }

    /// [Merge](`Entry::merge_by_ref`) every entry written to this formatter with the contents of `globals`.
    ///
    /// There is intentionally both a [`EntryIoStreamExt::merge_globals`] and a [`FormatExt::merge_globals`],
    /// which implement exactly the same functionality, to allow using in interfaces that accept
    /// an [`EntryIoStream`] as well as interfaces that accept a [`Format`].
    ///
    /// This helps users avoid having to store global, constant field values on every metric [`Entry`],
    /// for example "devops dimensions" like AvailabilityZone.
    /// ```
    /// # use metrique_writer::{
    /// #    Entry, EntryIoStream, EntryIoStreamExt,
    /// #    format::{FormatExt as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # use std::io;
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct Globals {
    ///    az: String
    /// }
    ///
    /// fn set_up_emf(out: impl io::Write) -> impl EntryIoStream {
    ///     Emf::all_validations("MyApp".into(), vec![vec![], vec!["az".into()]])
    ///         .merge_globals(Globals {
    ///             az: "us-east-1a".into(),
    ///         })
    ///         .output_to(out)
    /// }
    /// ```
    ///
    /// [`EntryIoStreamExt::merge_globals`]: crate::EntryIoStreamExt::merge_globals
    fn merge_globals<G>(self, globals: G) -> MergeGlobals<Self, G>
    where
        Self: Sized,
    {
        MergeGlobals {
            stream: self,
            globals,
        }
    }

    /// Adds a set of global dimensions to every metric of an entry except for those included in the
    /// `global_dimensions_denylist` as (class, instance) pairs.
    ///
    /// There is intentionally both a [`EntryIoStreamExt::merge_global_dimensions`] and a
    /// [`FormatExt::merge_global_dimensions`],  which implement exactly the same functionality,
    /// to allow using in interfaces that accept an [`EntryIoStream`] as well as interfaces
    /// that accept a [`Format`].
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    EntryIoStream,
    /// #    EntryIoStreamExt as _,
    /// #    format::{FormatExt as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # use smallvec::SmallVec;
    /// # use std::{borrow::Cow, collections::HashSet, io};
    ///
    /// fn set_up_emf(out: impl io::Write) -> impl EntryIoStream {
    ///     let mut global_dimensions: SmallVec<[(Cow<'_, str>, Cow<'_, str>); 1]> = SmallVec::with_capacity(1);
    ///     global_dimensions.push(("az".into(), "us-east-1a".into()));
    ///     let mut global_dimensions_denylist: HashSet<Cow<'_, str>> = HashSet::new();
    ///     global_dimensions_denylist.insert("ThisMetricWillBeEmittedWithoutAz".into());
    ///
    ///     Emf::all_validations("MyApp".into(), vec![vec![]])
    ///         .merge_global_dimensions(global_dimensions, Some(global_dimensions_denylist))
    ///         .output_to(out)
    /// }
    /// ```
    ///
    /// [`EntryIoStreamExt::merge_global_dimensions`]: crate::stream::EntryIoStreamExt::merge_global_dimensions
    /// [`FormatExt::merge_global_dimensions`]: crate::format::FormatExt::merge_global_dimensions
    fn merge_global_dimensions<const N: usize>(
        self,
        global_dimensions: SmallVec<[(CowStr, CowStr); N]>,
        global_dimensions_denylist: Option<HashSet<CowStr>>,
    ) -> MergeGlobalDimensions<Self, N>
    where
        Self: Sized,
    {
        MergeGlobalDimensions {
            stream: self,
            global_dimensions,
            global_dimensions_denylist: global_dimensions_denylist.unwrap_or_default(),
        }
    }
}
impl<T: Format + ?Sized> FormatExt for T {}

/// This struct combines a [Format] and an [std::io::Write]
/// to get an [EntryIoStream].
#[derive(Debug)]
pub struct FormattedEntryIoStream<F, O> {
    format: F,
    output: O,
}

impl<F: Format, O: io::Write> EntryIoStream for FormattedEntryIoStream<F, O> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        self.format.format(entry, &mut self.output)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

impl<F: Format, G: Entry> Format for MergeGlobals<F, G> {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        self.stream
            .format(&self.globals.merge_by_ref(entry), output)
    }
}

impl<F: Format, const N: usize> Format for MergeGlobalDimensions<F, N> {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        if self.global_dimensions.is_empty() {
            self.stream.format(&entry, output)
        } else {
            let entry_with_global_dimensions = WithGlobalDimensions::new(
                entry,
                self.global_dimensions.clone(),
                self.global_dimensions_denylist.clone(),
            );
            self.stream.format(&entry_with_global_dimensions, output)
        }
    }
}

#[derive(Debug)]
#[cfg(feature = "tracing_subscriber_03")]
/// This struct combines a [Format] and an [tracing_subscriber::fmt::MakeWriter]
/// to get an [EntryIoStream].
pub struct FormattedMakeWriterEntryIoStream<F, O> {
    format: F,
    output: O,
}

#[cfg(feature = "tracing_subscriber_03")]
impl<F: Format, O: for<'a> tracing_subscriber::fmt::MakeWriter<'a>> EntryIoStream
    for FormattedMakeWriterEntryIoStream<F, O>
{
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        self.format.format(entry, &mut self.output.make_writer())
    }

    fn flush(&mut self) -> io::Result<()> {
        // tracing-subscriber formatters do not need or support flushing
        Ok(())
    }
}
