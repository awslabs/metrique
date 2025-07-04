// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::{Entry, stream::IoStreamError};
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
}
