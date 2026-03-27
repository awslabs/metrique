// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "state")]
mod state;
#[cfg(feature = "state")]
pub use state::{LatestRef, State};

#[cfg(feature = "tokio-metrics-bridge")]
mod tokio_metrics_reporter;
#[cfg(feature = "tokio-metrics-bridge")]
pub use tokio_metrics_reporter::{AttachGlobalEntrySinkTokioMetricsExt, TokioRuntimeMetricsConfig};
