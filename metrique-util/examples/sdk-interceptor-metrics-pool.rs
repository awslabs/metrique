// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! AWS SDK interceptor pattern for request-local metrics with global fallback.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant, SystemTime};

use aws_sdk_cloudwatchlogs::Client as CloudWatchLogsClient;
use aws_sdk_cloudwatchlogs::config::{BehaviorVersion, Credentials, Region};
use aws_smithy_http_client::test_util::infallible_client_fn;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::interceptors::Intercept;
use aws_smithy_runtime_api::client::interceptors::context::{
    BeforeSerializationInterceptorContextRef, BeforeTransmitInterceptorContextRef,
    FinalizerInterceptorContextRef,
};
use aws_smithy_runtime_api::client::orchestrator::Metadata;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::body::SdkBody;
use aws_smithy_types::config_bag::{ConfigBag, Storable, StoreReplace};
use metrique::unit::Microsecond;
use metrique::unit_of_work::metrics;
use metrique::writer::sink::{DevNullSink, VecEntrySink};
use metrique::writer::{AttachGlobalEntrySink, GlobalEntrySink};
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
    /// Records that current-pool discovery selected the standalone fallback.
    metrics_pool_fallback: bool,
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
    CurrentOrStandalone,
    RequireCurrent,
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
            pool: PoolSelection::CurrentOrStandalone,
        }
    }

    fn qualifier(mut self, qualifier: &'static str) -> Self {
        self.qualifier = Some(qualifier);
        self
    }

    /// Require current-pool discovery to succeed.
    ///
    /// A missing pool triggers a debug assertion during development. Release
    /// builds warn and retain the standalone fallback so telemetry is not lost.
    fn require_current_pool(mut self) -> Self {
        self.pool = PoolSelection::RequireCurrent;
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

    /// `None` means no pool is reachable: either the application does not pool,
    /// or this interceptor is running detached from the request's scope, which a
    /// spawned task loses unless the spawn site captured a handle first. Callers
    /// cannot tell those apart, so `emit_invocation` falls back to a standalone
    /// entry rather than dropping the metric. The two paths are not
    /// interchangeable: the standalone entry carries its own timestamp and EMF
    /// configuration, which a pooled child must not.
    fn selected_pool(&self) -> Option<MetricsPoolHandle> {
        match &self.pool {
            PoolSelection::CurrentOrStandalone => MetricsPool::current(),
            PoolSelection::RequireCurrent => {
                let pool = MetricsPool::current();
                if pool.is_none() {
                    debug_assert!(
                        false,
                        "required current MetricsPool is unavailable; execution may have crossed a task spawn boundary"
                    );
                    metrique::writer::rate_limit::rate_limited!(
                        Duration::from_secs(60),
                        tracing::warn!(
                            service = self.service_name,
                            qualifier = self.qualifier.unwrap_or("None"),
                            "required current MetricsPool is unavailable; using standalone metrics fallback"
                        )
                    );
                }
                pool
            }
            PoolSelection::Explicit(pool) => Some(pool.clone()),
        }
    }
}

impl fmt::Display for SdkMetricsInterceptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SdkMetricsInterceptor")
    }
}

fn fake_cloudwatch_logs_client(interceptor: SdkMetricsInterceptor) -> CloudWatchLogsClient {
    let http_client = infallible_client_fn(|_request| {
        http::Response::builder()
            .status(200)
            .body(SdkBody::from(r#"{"logGroups":[]}"#))
            .unwrap()
    });
    let config = aws_sdk_cloudwatchlogs::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("test", "test", None, None, "example"))
        .http_client(http_client)
        .interceptor(interceptor)
        .build();
    CloudWatchLogsClient::from_conf(config)
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
                metrics_pool_fallback: true,
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
    // Standalone attempt metrics use the application's global sink. This
    // example focuses on the request-pooled invocation summary, so discard the
    // standalone records while the real SDK call still drives their emission.
    let _global_sink = ServiceMetrics::attach((DevNullSink::new(), ()));
    let sink = VecEntrySink::default();
    let request_metrics = RequestMetrics {
        operation: "ExampleOperation",
        metrics_pool: MetricsPool::new(),
    }
    .append_on_drop(sink);

    // The builder defaults to current-pool discovery with global fallback. The
    // real CloudWatch Logs client below uses an in-memory HTTP transport, so
    // `.send()` drives the full SDK interceptor lifecycle without making a
    // network request.
    let interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
        .qualifier("Logging")
        .build();
    let client = fake_cloudwatch_logs_client(interceptor);

    // Services that expect every invocation to be request-scoped can make a
    // missing current pool loud in development and observable in production.
    let _strict_interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
        .qualifier("Logging")
        .require_current_pool()
        .build();
    // A per-call interceptor can instead receive the pool explicitly.
    let _per_call_interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
        .qualifier("Logging")
        .metrics_pool(request_metrics.metrics_pool.handle())
        .build();

    let pool = request_metrics.metrics_pool.handle();
    let request_metrics = with_metrics_pool(pool, async move {
        client
            .describe_log_groups()
            .send()
            .await
            .expect("fake HTTP transport returns a valid response");
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

    #[tokio::test]
    async fn real_sdk_call_emits_interceptor_data_into_the_captured_pool() {
        let attempt_sink = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(attempt_sink.sink);
        let request = RequestMetrics {
            operation: "Request",
            metrics_pool: MetricsPool::new(),
        };
        let pool = request.metrics_pool.handle();
        let client = fake_cloudwatch_logs_client(
            SdkMetricsInterceptor::builder("CloudWatchLogs")
                .qualifier("Logging")
                .build(),
        );

        let request = with_metrics_pool(pool, async move {
            client.describe_log_groups().send().await.unwrap();
            request
        })
        .await;

        let request = test_metric(request);
        assert_eq!(
            request.values["SdkCloudWatchLogsLoggingOperation"],
            "DescribeLogGroups"
        );
        assert_eq!(request.values["SdkCloudWatchLogsLoggingStatusCode"], "200");
        assert_eq!(request.metrics["SdkCloudWatchLogsLoggingSuccess"], 1);
        assert_eq!(request.metrics["SdkCloudWatchLogsLoggingRetryCount"], 0);

        let attempts = attempt_sink.inspector.entries();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].values["Operation"], "DescribeLogGroups");
        assert_eq!(attempts[0].metrics["Success"], 1);
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
        assert_eq!(entries[0].metrics["MetricsPoolFallback"], 1);
    }

    #[test]
    fn default_current_selection_allows_a_missing_pool() {
        let interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs").build();
        assert!(interceptor.selected_pool().is_none());
    }

    #[test]
    fn required_current_pool_detects_a_missing_scope() {
        let interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
            .require_current_pool()
            .build();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| interceptor.selected_pool()));

        if cfg!(debug_assertions) {
            assert!(
                result.is_err(),
                "debug builds must assert on a missing pool"
            );
        } else {
            assert!(
                result
                    .expect("release builds must retain fallback")
                    .is_none(),
                "release fallback must report that no pool was selected"
            );
        }
    }

    #[tokio::test]
    async fn required_current_pool_accepts_an_installed_scope() {
        let pool = MetricsPool::new();
        let interceptor = SdkMetricsInterceptor::builder("CloudWatchLogs")
            .require_current_pool()
            .build();

        pool.handle()
            .scope(async move {
                assert!(interceptor.selected_pool().is_some());
            })
            .await;
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
