// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use metrique::ServiceMetrics;
use metrique::emf::Emf;
use metrique::writer::{AttachGlobalEntrySinkExt, FormatExt};
use metrique_util::{
    AttachGlobalEntrySinkTokioMetricsExt, MetricNameStyle, TokioRuntimeMetricsConfig,
};

const SAMPLING_INTERVAL: Duration = Duration::from_secs(1);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    let _attach_handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("TokioMetricsExample".to_string(), vec![vec![]])
            .output_to(std::io::stderr()),
    );

    let config = TokioRuntimeMetricsConfig::default()
        .with_interval(SAMPLING_INTERVAL)
        .with_name_style(MetricNameStyle::KebabCase);
    ServiceMetrics::subscribe_tokio_runtime_metrics(config);

    tokio::join![do_work(), do_work(), do_work()];

    Ok(())
}

async fn do_work() {
    for _ in 0..25 {
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
