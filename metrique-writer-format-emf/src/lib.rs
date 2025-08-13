// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod buf;
mod emf;
mod json_string;
mod rate_limit;

pub use emf::{
    AllowSplitEntries, Emf, EmfBuilder, EntryDimensions, HighStorageResolution,
    HighStorageResolutionCtor, MetricDefinition, MetricDirective, NoMetric, NoMetricCtor,
    SampledEmf, StorageResolution,
};
