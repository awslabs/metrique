// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding sysinfo metrics into per-request entries via a process-global
//! `OnceLock<State<SysinfoSnapshot>>`.
//!
//! Same shape as `sysinfo-folded.rs`, but the embed handle lives in a static
//! set at startup and cloned from inside `handle_request`. This is the typical
//! real-service pattern: the request handler doesn't need to carry the
//! sampler handle through its call chain, it just reads from the static.

use std::sync::OnceLock;
use std::time::Duration;

use metrique::{
    ServiceMetrics,
    emf::Emf,
    unit_of_work::metrics,
    writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink},
};
use metrique_util::{
    AttachGlobalEntrySinkSysinfoExt, State, SysinfoMetricsConfig, SysinfoSnapshot,
};

static SYSTEM_METRICS: OnceLock<State<SysinfoSnapshot>> = OnceLock::new();

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    success: bool,
    #[metrics(flatten, prefix = "sys_info_")]
    system: State<SysinfoSnapshot>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("FoldedSysinfoStaticExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    SYSTEM_METRICS
        .set(ServiceMetrics::embed_sysinfo_metrics(
            SysinfoMetricsConfig::default().with_interval(Duration::from_millis(500)),
        ))
        .ok();

    // The worker primes CPU counters and sleeps ~200ms before its first sample.
    // Without this wait, the first request would fold in the default zeros.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Each call to `handle_request` emits one EMF record carrying both the
    // user fields (`Operation`, `Success`) and the prefixed, folded system
    // fields (`SysInfoCpuUsage`, `SysInfoTotalMemory`, `SysInfoUsedMemory`,
    // `SysInfoUptime`, ...).
    for op in ["Read", "Write", "Read"] {
        handle_request(op).await;
    }

    Ok(())
}

async fn handle_request(op: &'static str) {
    let _m = RequestMetrics {
        operation: op,
        success: true,
        system: SYSTEM_METRICS
            .get()
            .expect("SYSTEM_METRICS initialized at startup")
            .clone(),
    }
    .append_on_drop(ServiceMetrics::sink());
    tokio::time::sleep(Duration::from_millis(100)).await;
}
