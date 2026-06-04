// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "state")]
mod state;
#[cfg(feature = "state")]
pub use state::{LatestRef, State};

#[cfg(any(feature = "tokio-metrics-bridge", feature = "sysinfo-bridge"))]
mod dynamic_inflection;

/// Metric field naming style shared by the bridge integrations
/// (`tokio-metrics-bridge`, `sysinfo-bridge`).
///
/// This is a re-export of [`metrique_core::DynamicNameStyle`].
#[cfg(any(feature = "tokio-metrics-bridge", feature = "sysinfo-bridge"))]
pub use metrique_core::DynamicNameStyle as MetricNameStyle;

#[cfg(feature = "tokio-metrics-bridge")]
mod tokio_metrics_reporter;
#[cfg(feature = "tokio-metrics-bridge")]
pub use tokio_metrics_reporter::{AttachGlobalEntrySinkTokioMetricsExt, TokioRuntimeMetricsConfig};

#[cfg(feature = "sysinfo-bridge")]
mod sysinfo_reporter;
#[cfg(feature = "sysinfo-bridge")]
pub use sysinfo_reporter::{
    AttachGlobalEntrySinkSysinfoExt, SysinfoMetrics, SysinfoMetricsConfig, SysinfoSnapshot,
};

#[cfg(feature = "pending-sink")]
#[cfg_attr(docsrs, doc(cfg(feature = "pending-sink")))]
pub mod pending_sink;
#[cfg(feature = "pending-sink")]
pub use pending_sink::PendingSinkResolver;
