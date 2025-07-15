// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

mod buf;
mod emf;
mod json_string;

pub use emf::{
    AllowSplitEntries, Emf, EmfBuilder, EntryDimensions, HighStorageResolution,
    HighStorageResolutionCtor, MetricDefinition, MetricDirective, NoMetric, NoMetricCtor,
    SampledEmf, StorageResolution,
};
