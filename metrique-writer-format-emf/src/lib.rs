// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod buf;
mod emf;
mod json_string;
mod rate_limit;

pub use emf::{
    AllowSplitEntries, Emf, EmfBuilder, EntryDimensions, HighStorageResolution,
    HighStorageResolutionCtor, MetricDefinition, MetricDirective, NoMetric, NoMetricCtor,
    SampledEmf, StorageResolution,
};

/// Re-exports of `FlagConstructor` types for use in `#[metrics(flags(...))]` attributes.
///
/// These provide ergonomic names for the flag types when used in descriptor attributes:
/// ```ignore
/// use metrique_writer_format_emf::flags::HighStorageResolution;
///
/// #[metrics(flags(HighStorageResolution))]
/// event_count: Counter,
/// ```
pub mod flags {
    pub use super::HighStorageResolutionCtor as HighStorageResolution;
    pub use super::NoMetricCtor as NoMetric;
}
