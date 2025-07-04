// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod buf;
mod emf;
mod json_string;

pub use emf::{
    AllowSplitEntries, Emf, EmfBuilder, EmfOptions, EntryDimensions, HighStorageResolution,
    HighStorageResolutionCtor, MetricDefinition, MetricDirective, NoMetric, NoMetricCtor,
    SampledEmf, StorageResolution,
};
