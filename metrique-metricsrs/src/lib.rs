// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub(crate) mod accumulator;
pub mod capture;
mod generic;
pub mod lambda_reporter;
pub mod metrics_histogram;
mod reporter;
mod unit;

pub use accumulator::{MetricAccumulatorEntry, MetricRecorder, SharedRecorder};
pub use generic::{MetricsRsVersion, ParametricRecorder};
pub use reporter::{MetricReporter, MetricReporterBuilder};
