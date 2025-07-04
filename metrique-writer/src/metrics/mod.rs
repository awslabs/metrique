// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This mode providesa few [`metrics::Recorder`]s that can be used for emitting metrics
//! via metrique-writer. This includes [`MetricsReporter`]  that is designed for use in EC2/Fargate,
//! [`lambda_reporter`] that is designed for use in Lambda, and [`capture`] that is
//! designed for use in unit tests.
//!
//! [`metrics::Recorder`]: metrics::Recorder
//! [`MetricsReporter`]: crate::metrics::MetricReporter
//! [`lambda_reporter`]: crate::metrics::lambda_reporter
//! [`capture`]: crate::metrics::capture

pub(crate) mod accumulator;
pub mod capture;
pub mod lambda_reporter;
pub mod metrics_histogram;
mod reporter;
mod unit;

pub use accumulator::{MetricAccumulatorEntry, MetricRecorder, SharedRecorder};
pub use reporter::{MetricReporter, MetricReporterBuilder};
