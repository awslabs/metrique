// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub use metrique_writer_core::entry::{BoxEntry, Entry, EntryConfig, EntryWriter};
pub use metrique_writer_core::global::GlobalEntrySink;
pub use metrique_writer_core::sink::{AnyEntrySink, BoxEntrySink, EntrySink};
pub use metrique_writer_core::stream::{EntryIoStream, IoStreamError};
pub use metrique_writer_core::unit::{Convert, Unit};
pub use metrique_writer_core::value::{MetricFlags, MetricValue, Observation, Value, ValueWriter};
pub use metrique_writer_core::{ValidationError, ValidationErrorBuilder};
pub use metrique_writer_macro::Entry;

pub use crate::sink::AttachGlobalEntrySinkExt;

pub mod entry;
pub mod format;
pub(crate) mod rate_limit;
pub mod sample;
pub mod sink;
pub mod stream;
#[cfg(feature = "test-util")]
pub mod test_util;
pub mod value;

#[cfg(all(feature = "metrics_rs_024", feature = "metrics_rs_bridge"))]
pub mod metrics;

#[doc(hidden)]
pub use metrique_writer_core as core;

pub use format::FormatExt;
pub use metrique_writer_core::global::AttachGlobalEntrySink;
pub use metrique_writer_core::unit;
pub use stream::EntryIoStreamExt;

pub(crate) type CowStr = std::borrow::Cow<'static, str>;
