// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use crate::entry::{BoxEntry, Entry, EntryConfig, EntryWriter};
pub use crate::global::GlobalEntrySink;
pub use crate::sample::SampleGroup;
pub use crate::sink::{AnyEntrySink, BoxEntrySink, EntrySink};
pub use crate::stream::{EntryIoStream, IoStreamError};
pub use crate::unit::{Convert, Unit};
pub use crate::validate::{ValidationError, ValidationErrorBuilder};
pub use crate::value::{MetricFlags, MetricValue, Observation, Value, ValueWriter};

pub(crate) type CowStr = std::borrow::Cow<'static, str>;

pub mod config;
pub mod entry;
pub mod format;
pub mod global;
pub mod sample;
pub mod sink;
pub mod stream;
pub mod unit;
mod validate;
pub mod value;

#[cfg(feature = "test-util")]
#[doc(hidden)]
pub use tokio as __tokio;

/// Private test module to make writing internal tests easier. This might change or
/// be fully removed in any version.
#[cfg(any(test, feature = "private-test-util"))]
#[doc(hidden)]
pub mod test_stream;
