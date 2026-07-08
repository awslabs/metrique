// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Attach the extra metric reporters (sysinfo + tokio runtime) to a shared sink.
//!
//! Each reporter runs on its own interval; both are aborted automatically when
//! the attach handle is dropped.

use std::time::Duration;

use metrique::ServiceMetrics;
use metrique::emf::Emf;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt};
use metrique_util::{
    AttachGlobalEntrySinkSysinfoExt, AttachGlobalEntrySinkTokioMetricsExt, MetricNameStyle,
    SysinfoMetricsConfig, TokioRuntimeMetricsConfig,
};

const SAMPLING_INTERVAL: Duration = Duration::from_secs(1);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("ExtrasExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    subscribe_sysinfo();
    subscribe_tokio_metrics();

    // Burn some CPU and allocate some memory so the sampled metrics move.
    tokio::join![do_work(), do_work(), do_work()];

    Ok(())
}

fn subscribe_sysinfo() {
    ServiceMetrics::subscribe_sysinfo_metrics(
        SysinfoMetricsConfig::default()
            .with_interval(SAMPLING_INTERVAL)
            .with_name_style(MetricNameStyle::KebabCase)
            .with_disks()
            .with_networks()
            .with_components(),
    );
}

fn subscribe_tokio_metrics() {
    ServiceMetrics::subscribe_tokio_runtime_metrics(
        TokioRuntimeMetricsConfig::default()
            .with_interval(SAMPLING_INTERVAL)
            .with_name_style(MetricNameStyle::KebabCase),
    );
}

async fn do_work() {
    let mut buf = Vec::with_capacity(1024 * 1024);
    for i in 0..25 {
        buf.extend(std::iter::repeat_n(i as u8, 1024 * 1024));
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    std::hint::black_box(buf);
}
