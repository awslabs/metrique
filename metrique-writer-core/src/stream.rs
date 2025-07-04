// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{fmt, io};

use crate::{Entry, ValidationError};

/// The error cases for a [`EntryIoStream::next`] call.
#[derive(Debug)]
pub enum IoStreamError {
    Validation(ValidationError),
    Io(io::Error),
}

impl fmt::Display for IoStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(err) => fmt::Display::fmt(err, f),
            Self::Io(err) => fmt::Display::fmt(err, f),
        }
    }
}

impl std::error::Error for IoStreamError {}

impl From<io::Error> for IoStreamError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ValidationError> for IoStreamError {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value)
    }
}

/// Writes a stream of [entries](`Entry`) to an output IO sink.
///
// Most applications should get an `EntryIoStream` by calling [`Format::output_to`](crate::format::Format::output_to)
//#[cfg_attr(
//    feature = "tracing_subscriber_03",
//    doc = "or [`Format::output_to_makewriter`](crate::format::Format::output_to_makewriter)"
//)]
// and possibly merging some global fields using [`EntryIoStream::merge_globals`].
///
/// Of course, if you have custom needs, it might be worth implementing this trait yourself.
///
/// Flushing may occur at any time, but is required to happen when [`EntryIoStream::flush`] is called.
pub trait EntryIoStream {
    /// Write the next [`Entry`] to the stream.
    ///
    /// If an [`IoStreamError::Io`] occurs, the result of the following call is undefined.
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError>;

    /// Flush any pending entries that have been written to a buffer before the final IO sink.
    ///
    /// Note that some writers like [`RotatingFileBuilder`](`crate::file::RotatingFileBuilder`) rely on regular flush
    /// calls to interleave IO operations that won't tear across entries.
    fn flush(&mut self) -> io::Result<()>;
}
