// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Folding sysinfo metrics into per-request entries.
//!
//! [`AttachGlobalEntrySinkSysinfoExt::embed_sysinfo_metrics`] starts a
//! background sampler and returns a `State<SysinfoSnapshot>` that can be
//! flattened into any entry, so every emitted record carries the latest
//! system sample.

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
        Emf::all_validations("FoldedSysinfoExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    let system = ServiceMetrics::embed_sysinfo_metrics(
        SysinfoMetricsConfig::default().with_interval(Duration::from_millis(500)),
    );

    // The worker primes CPU counters and sleeps ~200ms before its first sample.
    // Without this wait, the first request would fold in the default zeros.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Each loop iteration emits one EMF record like:
    //   {"Operation":"Read", "Success":1,
    //    "SysInfoTotalMemory":16777216000, "SysInfoUsedMemory":8123456789,
    //    "SysInfoCpuUsage":12.3, "SysInfoUptime":104537, ...}
    // — user fields and prefixed, folded system fields on the same line.
    for op in ["Read", "Write", "Read"] {
        let _m = RequestMetrics {
            operation: op,
            success: true,
            system: system.clone(),
        }
        .append_on_drop(ServiceMetrics::sink());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}
