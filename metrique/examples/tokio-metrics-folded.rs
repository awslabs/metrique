// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding Tokio runtime metrics into per-request entries.
//!
//! `subscribe_tokio_runtime_metrics` emits standalone runtime-metric entries
//! on an interval. This example shows how to also fold the latest sample
//! into per-request entries via [`EmbeddedTokioMetrics`], so each emitted
//! record carries runtime context without an extra join at query time.

use std::time::Duration;

use metrique::{
    ServiceMetrics,
    emf::Emf,
    unit_of_work::metrics,
    writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink},
};
use metrique_util::{
    AttachGlobalEntrySinkTokioMetricsExt, EmbeddedTokioMetrics, MetricNameStyle,
    TokioRuntimeMetricsConfig,
};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    success: bool,
    #[metrics(flatten)]
    runtime: EmbeddedTokioMetrics,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("FoldedTokioMetricsExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    // Spawns the background sampler that feeds `EmbeddedTokioMetrics`.
    ServiceMetrics::subscribe_tokio_runtime_metrics(
        TokioRuntimeMetricsConfig::default()
            .with_interval(Duration::from_millis(500))
            .with_name_style(MetricNameStyle::PascalCase),
    );

    // Without this, the first request would fold in the zero-default RuntimeMetrics.
    tokio::time::sleep(Duration::from_millis(500)).await;

    for op in ["Read", "Write", "Read"] {
        let _m = RequestMetrics {
            operation: op,
            success: true,
            runtime: EmbeddedTokioMetrics,
        }
        .append_on_drop(ServiceMetrics::sink());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}
