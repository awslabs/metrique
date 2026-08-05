// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! AWS SDK interceptor pattern for request-local metrics with global fallback.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant, SystemTime};

use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::interceptors::Intercept;
use aws_smithy_runtime_api::client::interceptors::context::{
    BeforeSerializationInterceptorContextRef, BeforeTransmitInterceptorContextRef,
    FinalizerInterceptorContextRef,
};
use aws_smithy_runtime_api::client::orchestrator::Metadata;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::config_bag::{ConfigBag, Storable, StoreReplace};
use metrique::unit::Microsecond;
use metrique::unit_of_work::metrics;
use metrique::writer::GlobalEntrySink;
use metrique::writer::sink::VecEntrySink;
use metrique::{ServiceMetrics, append_and_close};
use metrique_util::{MetricsPool, MetricsPoolHandle, with_metrics_pool};

#[derive(Debug, Clone)]
struct InvocationStart(Instant);

impl Storable for InvocationStart {
    type Storer = StoreReplace<Self>;
}

#[derive(Debug, Clone)]
struct AttemptStart(Instant);

impl Storable for AttemptStart {
    type Storer = StoreReplace<Self>;
}

#[derive(Debug, Clone)]
struct AttemptCount(Arc<AtomicU8>);

impl Storable for AttemptCount {
    type Storer = StoreReplace<Self>;
}

#[derive(Debug, Clone)]
struct CapturedPool(Option<MetricsPoolHandle>);

impl Storable for CapturedPool {
    type Storer = StoreReplace<Self>;
}

#[derive(Debug, Clone)]
#[metrics(subfield_owned)]
struct Attempt {
    sdk_service: &'static str,
    qualifier: &'static str,
    operation: String,
    outcome: &'static str,
    #[metrics(timestamp)]
    timestamp: SystemTime,
    #[metrics(unit = Microsecond)]
    api_call_duration: Option<Duration>,
    status_code: Option<String>,
    success: bool,
    error: bool,
    fault: bool,
    throttle: bool,
}

impl Storable for Attempt {
    type Storer = StoreReplace<Self>;
}

// Entry-level timestamp and EMF configuration stay on standalone entries; a
// pooled child contributes fields without replacing its parent entry metadata.
#[metrics(subfield_owned)]
struct PooledAttempt {
    sdk_service: &'static str,
    qualifier: &'static str,
    operation: String,
    outcome: &'static str,
    #[metrics(unit = Microsecond)]
    api_call_duration: Option<Duration>,
    status_code: Option<String>,
    success: bool,
    error: bool,
    fault: bool,
    throttle: bool,
}

impl From<&Attempt> for PooledAttempt {
    fn from(attempt: &Attempt) -> Self {
        Self {
            sdk_service: attempt.sdk_service,
            qualifier: attempt.qualifier,
            operation: attempt.operation.clone(),
            outcome: attempt.outcome,
            api_call_duration: attempt.api_call_duration,
            status_code: attempt.status_code.clone(),
            success: attempt.success,
            error: attempt.error,
            fault: attempt.fault,
            throttle: attempt.throttle,
        }
    }
}

#[metrics(subfield_owned)]
struct PooledSdkInvocationMetrics {
    #[metrics(flatten)]
    final_attempt: PooledAttempt,
    #[metrics(unit = Microsecond)]
    invocation_duration: Option<Duration>,
    retry_count: u8,
    level: &'static str,
}

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [
        ["SdkService", "Level"],
        ["SdkService", "Qualifier", "Level"]
    ]
)]
struct StandaloneSdkInvocationMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    #[metrics(flatten)]
    invocation: PooledSdkInvocationMetrics,
}

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [
        ["SdkService", "Level"],
        ["SdkService", "Qualifier", "Level"]
    ]
)]
struct StandaloneSdkAttemptMetrics {
    #[metrics(flatten)]
    attempt: Attempt,
    level: &'static str,
}

#[derive(Debug, Clone)]
enum PoolSelection {
    Current,
    Explicit(MetricsPoolHandle),
}

#[derive(Debug, Clone)]
struct SdkMetricsInterceptor {
    service_name: &'static str,
    qualifier: Option<&'static str>,
    pool: PoolSelection,
}

#[derive(Debug)]
struct SdkMetricsInterceptorBuilder {
    service_name: &'static str,
    qualifier: Option<&'static str>,
    pool: PoolSelection,
}

impl SdkMetricsInterceptorBuilder {
    fn new(service_name: &'static str) -> Self {
        Self {
            service_name,
            qualifier: None,
            pool: PoolSelection::Current,
        }
    }

    fn qualifier(mut self, qualifier: &'static str) -> Self {
        self.qualifier = Some(qualifier);
        self
    }

    fn metrics_pool(mut self, pool: MetricsPoolHandle) -> Self {
        self.pool = PoolSelection::Explicit(pool);
        self
    }

    fn build(self) -> SdkMetricsInterceptor {
        SdkMetricsInterceptor {
            service_name: self.service_name,
            qualifier: self.qualifier,
            pool: self.pool,
        }
    }
}

impl SdkMetricsInterceptor {
    fn builder(service_name: &'static str) -> SdkMetricsInterceptorBuilder {
        SdkMetricsInterceptorBuilder::new(service_name)
    }

    fn selected_pool(&self) -> Option<MetricsPoolHandle> {
        match &self.pool {
            PoolSelection::Current => MetricsPool::current(),
            PoolSelection::Explicit(pool) => Some(pool.clone()),
        }
    }
}

impl fmt::Display for SdkMetricsInterceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SdkMetricsInterceptor")
    }
}

fn emit_invocation(
    pool: Option<&MetricsPoolHandle>,
    timestamp: SystemTime,
    invocation: PooledSdkInvocationMetrics,
) {
    if let Some(pool) = pool {
        pool.append(invocation);
    } else {
        // Background tasks and other callers outside a request scope retain the
        // existing standalone emission behavior.
        drop(append_and_close(
            StandaloneSdkInvocationMetrics {
                timestamp,
                invocation,
            },
            ServiceMetrics::sink(),
        ));
    }
}

impl Intercept for SdkMetricsInterceptor {
    fn name(&self) -> &'static str {
        "SdkMetricsInterceptor"
    }

    fn read_before_execution(
        &self,
        _context: &BeforeSerializationInterceptorContextRef<'_>,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        cfg.interceptor_state()
            .store_put(InvocationStart(Instant::now()));
        cfg.interceptor_state()
            .store_put(AttemptCount(Arc::new(AtomicU8::new(0))));

        // Capture once so all retries and finalization use the destination selected
        // when this invocation began.
        let pool = self.selected_pool().map(|pool| {
            let pool = pool.with_prefix(["sdk", self.service_name]);
            match self.qualifier {
                Some(qualifier) => pool.with_prefix([qualifier]),
                None => pool,
            }
        });
        cfg.interceptor_state().store_put(CapturedPool(pool));
        Ok(())
    }

    fn read_before_attempt(
        &self,
        _context: &BeforeTransmitInterceptorContextRef<'_>,
        _runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        cfg.interceptor_state()
            .store_put(AttemptStart(Instant::now()));
        if let Some(count) = cfg.load::<AttemptCount>() {
            count.0.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn read_after_attempt(
        &self,
        context: &FinalizerInterceptorContextRef<'_>,
        _runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let operation = cfg
            .load::<Metadata>()
            .map_or("Unknown", |metadata| metadata.name())
            .to_string();
        let api_call_duration = cfg.load::<AttemptStart>().map(|start| start.0.elapsed());
        let success = matches!(context.output_or_error(), Some(Ok(_)));
        let status = context.response().map(|response| response.status());
        let attempt = Attempt {
            sdk_service: self.service_name,
            qualifier: self.qualifier.unwrap_or("None"),
            operation,
            outcome: if success { "Pass" } else { "Fail" },
            timestamp: SystemTime::now(),
            api_call_duration,
            status_code: status.map(|status| status.as_u16().to_string()),
            success,
            error: status.is_some_and(|status| status.is_client_error()),
            fault: status.is_some_and(|status| status.is_server_error()),
            // The production interceptor retains its SDK retry-classifier helper here.
            throttle: false,
        };

        // Phoenix keeps one standalone entry per attempt. Only the final
        // invocation summary is folded into the request metric.
        drop(append_and_close(
            StandaloneSdkAttemptMetrics {
                attempt: attempt.clone(),
                level: "Attempt",
            },
            ServiceMetrics::sink(),
        ));
        cfg.interceptor_state().store_put(attempt);
        Ok(())
    }

    fn read_after_execution(
        &self,
        _context: &FinalizerInterceptorContextRef<'_>,
        _runtime_components: &RuntimeComponents,
        cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let Some(last_attempt) = cfg.load::<Attempt>() else {
            return Ok(());
        };

        let attempts = cfg
            .load::<AttemptCount>()
            .map(|count| count.0.load(Ordering::Relaxed))
            .unwrap_or(1);
        let invocation = PooledSdkInvocationMetrics {
            final_attempt: last_attempt.into(),
            invocation_duration: cfg.load::<InvocationStart>().map(|start| start.0.elapsed()),
            retry_count: attempts.saturating_sub(1),
            level: "Invocation",
        };

        let pool = cfg
            .load::<CapturedPool>()
            .and_then(|captured| captured.0.as_ref());
        emit_invocation(pool, last_attempt.timestamp, invocation);
        Ok(())
    }
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(flatten)]
    metrics_pool: MetricsPool,
}

#[tokio::main]
async fn main() {
    let sink = VecEntrySink::default();
    let request_metrics = RequestMetrics {
        operation: "ExampleOperation",
        metrics_pool: MetricsPool::new(),
    }
    .append_on_drop(sink);

    // The builder defaults to current-pool discovery with global fallback.
    let _interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
        .qualifier("Logging")
        .build();
    // A per-call interceptor can instead receive the pool explicitly.
    let _per_call_interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
        .qualifier("Logging")
        .metrics_pool(request_metrics.metrics_pool.handle())
        .build();

    // In a service, the SDK client's `.send().await` runs inside this future.
    let pool = request_metrics.metrics_pool.handle();
    let request_metrics = with_metrics_pool(pool, async move {
        // client.put_log_events().send().await?;
        request_metrics
    })
    .await;

    drop(request_metrics);
}

#[cfg(test)]
mod tests {
    use metrique::test_util::{test_entry_sink, test_metric};

    use super::*;

    fn invocation(operation: &str, retries: u8) -> PooledSdkInvocationMetrics {
        PooledSdkInvocationMetrics {
            final_attempt: PooledAttempt {
                sdk_service: "CloudWatchLogs",
                qualifier: "Logging",
                operation: operation.to_string(),
                outcome: "Pass",
                api_call_duration: Some(Duration::from_micros(20)),
                status_code: Some("200".to_string()),
                success: true,
                error: false,
                fault: false,
                throttle: false,
            },
            invocation_duration: Some(Duration::from_micros(50)),
            retry_count: retries,
            level: "Invocation",
        }
    }

    #[test]
    fn invocation_uses_the_captured_pool() {
        let request = RequestMetrics {
            operation: "Request",
            metrics_pool: MetricsPool::new(),
        };
        let pool = request
            .metrics_pool
            .handle()
            .with_prefix(["sdk", "cloudwatch_logs", "logging"]);

        emit_invocation(
            Some(&pool),
            SystemTime::now(),
            invocation("PutLogEvents", 2),
        );

        let request = test_metric(request);
        assert_eq!(
            request.values["SdkCloudwatchLogsLoggingOperation"],
            "PutLogEvents"
        );
        assert_eq!(request.metrics["SdkCloudwatchLogsLoggingRetryCount"], 2);
    }

    #[test]
    fn invocation_falls_back_to_the_global_sink() {
        let sink = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(sink.sink);

        emit_invocation(None, SystemTime::now(), invocation("PutLogEvents", 3));

        let entries = sink.inspector.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].values["Operation"], "PutLogEvents");
        assert_eq!(entries[0].metrics["RetryCount"], 3);
    }

    #[test]
    fn explicit_pool_does_not_require_a_scope() {
        let request = RequestMetrics {
            operation: "Request",
            metrics_pool: MetricsPool::new(),
        };
        let interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
            .qualifier("Logging")
            .metrics_pool(request.metrics_pool.handle())
            .build();

        let pool = interceptor.selected_pool().unwrap().with_prefix(["sdk"]);
        emit_invocation(
            Some(&pool),
            SystemTime::now(),
            invocation("PutLogEvents", 1),
        );

        let request = test_metric(request);
        assert_eq!(request.values["SdkOperation"], "PutLogEvents");
        assert_eq!(request.metrics["SdkRetryCount"], 1);
    }
}
