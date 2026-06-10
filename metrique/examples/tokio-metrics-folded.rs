// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding Tokio runtime metrics into per-request entries.
//!
//! [`AttachGlobalEntrySinkTokioMetricsExt::embed_tokio_runtime_metrics`]
//! starts a background sampler and returns a `State<TokioRuntimeSnapshot>`
//! that can be flattened into any entry, so every emitted record carries
//! the latest runtime sample.

use std::time::Duration;

use metrique::{
    ServiceMetrics,
    emf::Emf,
    unit_of_work::metrics,
    writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink},
};
use metrique_util::{
    AttachGlobalEntrySinkTokioMetricsExt, State, TokioRuntimeMetricsConfig, TokioRuntimeSnapshot,
};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    success: bool,
    #[metrics(flatten, prefix = "tokio_")]
    runtime: State<TokioRuntimeSnapshot>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("FoldedTokioMetricsExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    let runtime = ServiceMetrics::embed_tokio_runtime_metrics(
        TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(500)),
    );

    // Each loop iteration emits one EMF record like:
    //   {"Operation":"Read", "Success":1,
    //    "TokioWorkersCount":12, "TokioTotalParkCount":4, "TokioTotalBusyDuration":0.135,
    //    "TokioGlobalQueueDepth":0, "TokioElapsed":500.7, ...}
    // — user fields and folded runtime fields on the same line.
    for op in ["Read", "Write", "Read"] {
        let _m = RequestMetrics {
            operation: op,
            success: true,
            runtime: runtime.clone(),
        }
        .append_on_drop(ServiceMetrics::sink());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}
