// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains various utilities for working with [EntryIoStream]

use std::{collections::HashSet, io};

use metrique_writer_core::{Entry, config::BasicErrorMessage};
use smallvec::SmallVec;

use crate::{CowStr, entry::WithGlobalDimensions};

pub use metrique_writer_core::{EntryIoStream, IoStreamError};

/// Extension trait for [`EntryIoStream`]. This adds methods that use types not
/// present within [`metrique_writer_core`].
pub trait EntryIoStreamExt: EntryIoStream {
    /// [Merge](`Entry::merge_by_ref`) every entry written to this stream with the contents of `globals`.
    ///
    /// There is intentionally both a [`EntryIoStreamExt::merge_globals`] and a
    /// [`FormatExt::merge_globals`],  which implement exactly the same functionality,
    /// to allow using in interfaces that accept an [`EntryIoStream`] as well as interfaces
    /// that accept a [`Format`].
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
    ///         .output_to(out)
    ///         .merge_globals(Globals {
    ///             az: "us-east-1a".into(),
    ///         })
    /// }
    /// ```
    ///
    /// [`Format`]: crate::format::Format
    /// [`FormatExt::merge_globals`]: crate::format::FormatExt::merge_globals
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
    ///         .output_to(out)
    ///         .merge_global_dimensions(global_dimensions, Some(global_dimensions_denylist))
    /// }
    /// ```
    ///
    /// [`Format`]: crate::format::Format
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

    /// See [`tee()`].
    fn tee<S>(self, other: S) -> Tee<Self, S>
    where
        Self: Sized,
    {
        tee(self, other)
    }

    /// Report an error message to the relevant log streams in the correct format
    ///
    /// This function uses [EntryIoStream::next_basic_error] to be able of reporting
    /// an error even if some global dimensions are invalid.
    fn report_error(&mut self, message: &str) -> Result<(), IoStreamError> {
        self.next(&BasicErrorMessage::new(message))
    }
}
impl<T: EntryIoStream + ?Sized> EntryIoStreamExt for T {}

/// Create a new [`EntryIoStream`] that writes each incoming entry to both `s1` and `s2`.
///
/// This helps replicate metrics across different formats during a migration or for creating a durable log paired with
/// a sampled log for metrics.
/// ```
/// # use metrique_writer::{
/// #    EntryIoStream,
/// #    format::{FormatExt as _},
/// #    stream::tee,
/// #    sample::{SampledFormat, SampledFormatExt as _ },
/// # };
/// # use metrique_writer_format_emf::Emf;
/// # use std::io;
/// # use std::path::Path;
///
/// use tracing_appender::rolling::{RollingFileAppender, Rotation};
///
/// // Keep a durable log under `service_log` of all requests, but also output a sampled log for metric publishing via
/// // CloudWatch. This second log is sampled to reduce load at a small cost to accuracy.
/// fn set_up_emf(log_dir: impl AsRef<Path>) -> impl EntryIoStream {
///     tee(
///         Emf::no_validations("MyApp".into(), vec![vec![]])
///             .output_to_makewriter(
///                     RollingFileAppender::new(Rotation::HOURLY, &log_dir, "service_log.log")
///             ),
///         Emf::all_validations("MyApp".into(), vec![vec![]])
///             .with_sampling()
///             .sample_by_fixed_fraction(0.1)
///             .output_to_makewriter(
///                     RollingFileAppender::new(Rotation::HOURLY, &log_dir, "metric_log.log")
///             ),
///     )
/// }
/// ```
pub fn tee<S1, S2>(s1: S1, s2: S2) -> Tee<S1, S2> {
    Tee { s1, s2 }
}

/// See [`tee()`].
#[derive(Debug)]
pub struct Tee<S1, S2> {
    s1: S1,
    s2: S2,
}

impl<S1: EntryIoStream, S2: EntryIoStream> EntryIoStream for Tee<S1, S2> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        self.s1.next(entry).and(self.s2.next(entry))
    }

    fn flush(&mut self) -> io::Result<()> {
        let r1 = self.s1.flush();
        let r2 = self.s2.flush();
        r1.and(r2)
    }
}

/// See [`EntryIoStreamExt::merge_globals`] or [`FormatExt::merge_globals`].
///
/// [`FormatExt::merge_globals`]: crate::format::FormatExt::merge_globals
#[derive(Clone)]
pub struct MergeGlobals<S, G> {
    pub(crate) stream: S,
    pub(crate) globals: G,
}

impl<S: EntryIoStream, G: Entry> EntryIoStream for MergeGlobals<S, G> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        self.stream.next(&self.globals.merge_by_ref(entry))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// See [`EntryIoStreamExt::merge_global_dimensions`] or [`FormatExt::merge_global_dimensions`].
///
/// [`EntryIoStreamExt::merge_global_dimensions`]: crate::stream::EntryIoStreamExt::merge_global_dimensions
/// [`FormatExt::merge_global_dimensions`]: crate::format::FormatExt::merge_global_dimensions
#[derive(Clone)]
pub struct MergeGlobalDimensions<S, const N: usize> {
    pub(crate) stream: S,
    pub(crate) global_dimensions: SmallVec<[(CowStr, CowStr); N]>,
    pub(crate) global_dimensions_denylist: HashSet<CowStr>,
}

impl<S: EntryIoStream, const N: usize> EntryIoStream for MergeGlobalDimensions<S, N> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        if self.global_dimensions.is_empty() {
            self.stream.next(&entry)
        } else {
            let entry_with_global_dimensions = WithGlobalDimensions::new(
                entry,
                self.global_dimensions.clone(),
                self.global_dimensions_denylist.clone(),
            );
            self.stream.next(&entry_with_global_dimensions)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// An EntryIoStream that drops all entries sent to it
#[derive(Default, Copy, Clone, Debug)]
#[non_exhaustive]
pub struct NullEntryIoStream;

impl EntryIoStream for NullEntryIoStream {
    fn next(&mut self, _entry: &impl Entry) -> Result<(), IoStreamError> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
