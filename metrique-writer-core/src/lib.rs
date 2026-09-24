// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use crate::descriptor::{
    AvailableDescriptors, DescriptorId, DescriptorIter, DescriptorRef, Descriptors,
    EntryDescriptor, EntryDescriptorBuilder, FieldDescriptor, FieldDescriptorBuilder, FieldFlag,
    FieldShape, FieldView, KnownShape, ShapeRef, StringShape, Style, Styles, TimestampDescriptor,
};
pub use crate::entry::{BoxEntry, Entry, EntryConfig, EntryWriter};
pub use crate::global::GlobalEntrySink;
pub use crate::sample::SampleGroup;
pub use crate::sink::{AnyEntrySink, BoxEntrySink, EntrySink};
pub use crate::stream::{EntryIoStream, IoStreamError};
pub use crate::unit::{Convert, Unit};
pub use crate::validate::{ValidationError, ValidationErrorBuilder};
pub use crate::value::{Distribution, MetricFlags, MetricValue, Observation, Value, ValueWriter};

pub(crate) type CowStr = std::borrow::Cow<'static, str>;

pub mod config;
pub mod descriptor;
pub mod entry;
pub mod format;
pub mod global;

/// Canonical JSON scalar encoders shared between core's object string-fallback and the
/// `metrique-writer-format-json` native renderer. Public only so the format crate can call
/// into it; hidden from rustdoc but semver-relevant (see the module docs).
#[doc(hidden)]
pub mod json_encode;
pub mod quantize;
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

/// Shuttle-only test support shared across this workspace. This might
/// change or be fully removed in any version.
#[cfg(all(shuttle, feature = "_shuttle"))]
#[doc(hidden)]
pub mod shuttle_test_support;

/// Cfg-gated concurrency primitives (std vs. shuttle) used by macro-generated
/// code. Public only so that expansion works from downstream crates; not
/// meant to be used directly.
#[doc(hidden)]
pub mod primitives;
