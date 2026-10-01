// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "state")]
mod state;
#[cfg(feature = "state")]
pub use state::{LatestRef, State};

#[cfg(feature = "metrics-pool")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics-pool")))]
pub mod metrics_pool;
#[cfg(feature = "metrics-pool")]
#[doc(inline)]
pub use metrics_pool::{
    MetricsPool, MetricsPoolBuilder, MetricsPoolHandle, MetricsPoolScope, propagate_current,
    with_metrics_pool,
};
// Named by `<MetricsPool as CloseValue>::Closed`, so it must be nameable from
// downstream crates even though it is not part of the documented surface.
#[cfg(feature = "metrics-pool")]
#[doc(hidden)]
pub use metrics_pool::MetricsPoolEntry;

#[cfg(any(
    feature = "metrics-pool",
    feature = "tokio-metrics-bridge",
    feature = "sysinfo-bridge"
))]
mod dynamic_inflection;

/// Metric field naming style shared by the runtime-inflected integrations
/// (`metrics-pool`, `tokio-metrics-bridge`, `sysinfo-bridge`).
///
/// These integrations choose the field-name inflection at runtime rather than
/// at compile time, so they share this style enum.
///
/// This is a re-export of [`metrique_core::DynamicNameStyle`].
#[cfg(any(
    feature = "metrics-pool",
    feature = "tokio-metrics-bridge",
    feature = "sysinfo-bridge"
))]
pub use metrique_core::DynamicNameStyle as MetricNameStyle;

#[cfg(feature = "tokio-metrics-bridge")]
mod tokio_metrics_reporter;
#[cfg(feature = "tokio-metrics-bridge")]
pub use tokio_metrics_reporter::{
    AttachGlobalEntrySinkTokioMetricsExt, TokioRuntimeMetricsConfig, TokioRuntimeSnapshot,
};

#[cfg(feature = "tokio-metrics-bridge")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio-metrics-bridge")))]
pub mod future_metrics;

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
