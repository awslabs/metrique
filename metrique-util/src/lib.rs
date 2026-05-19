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
mod dynamic_inflection;
#[cfg(feature = "tokio-metrics-bridge")]
mod tokio_metrics_reporter;
#[cfg(feature = "tokio-metrics-bridge")]
pub use tokio_metrics_reporter::{
    AttachGlobalEntrySinkTokioMetricsExt, EmbeddedTokioMetrics, MetricNameStyle,
    TokioRuntimeMetricsConfig,
};

#[cfg(feature = "pending-sink")]
#[cfg_attr(docsrs, doc(cfg(feature = "pending-sink")))]
pub mod pending_sink;
#[cfg(feature = "pending-sink")]
pub use pending_sink::PendingSinkResolver;
