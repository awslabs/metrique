// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains the [`Format`] trait.

use crate::{Entry, config::BasicErrorMessage, stream::IoStreamError};
use std::io;

/// Describes how to format [entries](`Entry`) to a stream of bytes.
pub trait Format {
    /// Core format API that writes `entry` to the given `output` IO destination.
    ///
    /// Note that no assumptions should be made about `output` being the same IO destination between successive calls
    /// to `format`. For example, a rotating file may be swapped out in between calls.
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError>;

    /// Write a basic error message to the stream.
    /// This should be used if printing even a basic entry causes validation errors,
    /// to provide at least some indication of an error even if tracing is disabled.
    ///
    /// Formats that want good usability should ensure that formatting a [BasicErrorMessage]
    /// is possible. The formatted message might not be routed properly since
    /// it might lack routing information (for example, dimensions or namespaces),
    /// but it should follow the framing format and be human-visible for someone
    /// observing the stream.
    fn format_basic_error(
        &mut self,
        message: &str,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        self.format(&BasicErrorMessage::new(message), output)
    }
}
