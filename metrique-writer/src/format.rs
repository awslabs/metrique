// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::io;

use metrique_writer_core::{
    Entry,
    stream::{EntryIoStream, IoStreamError},
};

pub use metrique_writer_core::format::Format;

pub trait FormatExt: Format {
    /// Bind the format to an `output` IO destination to create an [`EntryIoStream`].
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
}
impl<T: Format + ?Sized> FormatExt for T {}

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

#[derive(Debug)]
#[cfg(feature = "tracing_subscriber_03")]
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
