// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding Tokio runtime metrics into per-request entries via a process-global
//! `OnceLock<State<TokioRuntimeSnapshot>>`.
//!
//! Same shape as `tokio-metrics-folded.rs`, but the embed handle lives in a
//! static set at startup and cloned from inside `handle_request`. This is the
//! typical real-service pattern: the request handler doesn't need to carry
//! the sampler handle through its call chain, it just reads from the static.

use std::sync::OnceLock;
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

static RUNTIME_METRICS: OnceLock<State<TokioRuntimeSnapshot>> = OnceLock::new();

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    success: bool,
    #[metrics(flatten)]
    runtime: State<TokioRuntimeSnapshot>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("FoldedTokioMetricsStaticExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    RUNTIME_METRICS
        .set(ServiceMetrics::embed_tokio_runtime_metrics(
            TokioRuntimeMetricsConfig::default().with_interval(Duration::from_millis(500)),
        ))
        .ok();

    // Each call to `handle_request` emits one EMF record carrying both the
    // user fields (`Operation`, `Success`) and the folded runtime fields
    // (`WorkersCount`, `TotalParkCount`, `Elapsed`, ...).
    for op in ["Read", "Write", "Read"] {
        handle_request(op).await;
    }

    Ok(())
}

async fn handle_request(op: &'static str) {
    let _m = RequestMetrics {
        operation: op,
        success: true,
        runtime: RUNTIME_METRICS
            .get()
            .expect("RUNTIME_METRICS initialized at startup")
            .clone(),
    }
    .append_on_drop(ServiceMetrics::sink());
    tokio::time::sleep(Duration::from_millis(100)).await;
}
