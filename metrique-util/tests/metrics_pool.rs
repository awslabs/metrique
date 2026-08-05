// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique::writer::core::Descriptors;
use metrique::writer::test_util::{test_metric, to_test_entry};
use metrique::writer::{Entry, EntryWriter};
use metrique::{CloseValue, InflectableEntry, PascalCase};
use metrique_util::{MetricsPool, with_metrics_pool};

#[metrics]
#[derive(Default)]
struct SdkInvocationMetrics {
    operation: &'static str,
    retry_count: u64,
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(flatten)]
    metrics_pool: MetricsPool,
    request_count: u64,
}

#[metrics]
struct IdentityRequestMetrics {
    #[metrics(flatten)]
    metrics_pool: MetricsPool,
}

#[metrics(rename_all = "snake_case")]
struct SnakeRequestMetrics {
    #[metrics(flatten)]
    metrics_pool: MetricsPool,
}

#[metrics(rename_all = "kebab-case")]
struct KebabRequestMetrics {
    #[metrics(flatten)]
    metrics_pool: MetricsPool,
}

#[metrics(rename_all = "PascalCase")]
struct PrefixedRequestMetrics {
    #[metrics(flatten, prefix = "request_")]
    metrics_pool: MetricsPool,
}

#[metrics(rename_all = "PascalCase")]
struct ExactPrefixedRequestMetrics {
    #[metrics(flatten, exact_prefix = "sdk.request.")]
    metrics_pool: MetricsPool,
}

async fn handle_request(metrics: &mut RequestMetrics) {
    MetricsPool::current()
        .expect("request pool is installed")
        .with_prefix(["sdk", "cloudwatch_logs"])
        .append(SdkInvocationMetrics {
            operation: "PutLogEvents",
            retry_count: 2,
        });
    metrics.request_count += 1;
}

struct PascalEntry<M>(M);

impl<M: InflectableEntry<PascalCase>> Entry for PascalEntry<M> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        InflectableEntry::<PascalCase>::write(&self.0, writer);
    }

    fn descriptors(&self) -> Descriptors<'_> {
        InflectableEntry::<PascalCase>::descriptors(&self.0)
    }
}

#[tokio::test]
async fn scope_function_accepts_an_owned_handle() {
    let mut metrics = RequestMetrics {
        metrics_pool: MetricsPool::new(),
        request_count: 0,
    };
    let pool = metrics.metrics_pool.handle();

    let metrics = with_metrics_pool(pool, async move {
        handle_request(&mut metrics).await;
        metrics
    })
    .await;

    let entry = test_metric(metrics);
    check!(entry.metrics["RequestCount"] == 1);
    check!(entry.values["SdkCloudwatchLogsOperation"] == "PutLogEvents");
    check!(entry.metrics["SdkCloudwatchLogsRetryCount"] == 2);
}

#[test]
fn current_pool_is_absent_outside_a_scope() {
    check!(MetricsPool::current().is_none());
}

#[test]
fn pooled_metrics_follow_the_parent_name_style() {
    let identity = IdentityRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    identity
        .metrics_pool
        .handle()
        .with_prefix(["sdk", "request"])
        .append(SdkInvocationMetrics {
            operation: "Identity",
            retry_count: 1,
        });
    let identity = test_metric(identity);
    check!(identity.values["sdk_request_operation"] == "Identity");
    check!(identity.metrics["sdk_request_retry_count"] == 1);

    let snake = SnakeRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    snake
        .metrics_pool
        .handle()
        .with_prefix(["sdk", "request"])
        .append(SdkInvocationMetrics {
            operation: "Snake",
            retry_count: 2,
        });
    let snake = test_metric(snake);
    check!(snake.values["sdk_request_operation"] == "Snake");
    check!(snake.metrics["sdk_request_retry_count"] == 2);

    let kebab = KebabRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    kebab
        .metrics_pool
        .handle()
        .with_prefix(["sdk", "request"])
        .append(SdkInvocationMetrics {
            operation: "Kebab",
            retry_count: 3,
        });
    let kebab = test_metric(kebab);
    check!(kebab.values["sdk-request-operation"] == "Kebab");
    check!(kebab.metrics["sdk-request-retry-count"] == 3);
}

#[test]
fn pool_preserves_flatten_site_prefixes() {
    let prefixed = PrefixedRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    prefixed
        .metrics_pool
        .handle()
        .with_prefix(["sdk"])
        .append(SdkInvocationMetrics {
            operation: "Inflected",
            retry_count: 1,
        });
    let prefixed = test_metric(prefixed);
    check!(prefixed.values["RequestSdkOperation"] == "Inflected");

    let exact = ExactPrefixedRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    exact.metrics_pool.handle().append(SdkInvocationMetrics {
        operation: "Exact",
        retry_count: 2,
    });
    let exact = test_metric(exact);
    check!(exact.values["sdk.request.Operation"] == "Exact");
    check!(exact.metrics["sdk.request.RetryCount"] == 2);
}

#[test]
fn exact_handle_prefix_supports_dot_delimited_names() {
    let request = IdentityRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    request
        .metrics_pool
        .handle()
        .with_exact_prefix("sdk.request.")
        .append(SdkInvocationMetrics {
            operation: "Dotted",
            retry_count: 3,
        });

    let request = test_metric(request);
    check!(request.values["sdk.request.operation"] == "Dotted");
    check!(request.metrics["sdk.request.retry_count"] == 3);
}

#[test]
fn later_pooled_metrics_overwrite_colliding_fields() {
    let pool = MetricsPool::new();
    let handle = pool.handle().with_prefix(["sdk"]);
    handle.append(SdkInvocationMetrics {
        operation: "First",
        retry_count: 1,
    });
    handle.append(SdkInvocationMetrics {
        operation: "Second",
        retry_count: 2,
    });

    let closed = pool.close();
    check!(matches!(
        InflectableEntry::<PascalCase>::descriptors(&closed),
        Descriptors::Unavailable
    ));

    let entry = to_test_entry(PascalEntry(closed));
    check!(entry.values["SdkOperation"] == "Second");
    check!(entry.metrics["SdkRetryCount"] == 2);
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_scopes_are_isolated_on_one_thread() {
    let first = RequestMetrics {
        metrics_pool: MetricsPool::new(),
        request_count: 1,
    };
    let first_pool = first.metrics_pool.handle();
    let first = first_pool.scope(async move {
        tokio::task::yield_now().await;
        MetricsPool::current()
            .unwrap()
            .with_prefix(["sdk"])
            .append(SdkInvocationMetrics {
                operation: "First",
                retry_count: 1,
            });
        first
    });

    let second = RequestMetrics {
        metrics_pool: MetricsPool::new(),
        request_count: 2,
    };
    let second_pool = second.metrics_pool.handle();
    let second = second_pool.scope(async move {
        MetricsPool::current()
            .unwrap()
            .with_prefix(["sdk"])
            .append(SdkInvocationMetrics {
                operation: "Second",
                retry_count: 2,
            });
        tokio::task::yield_now().await;
        second
    });

    let (first, second) = tokio::join!(first, second);
    let first = test_metric(first);
    let second = test_metric(second);

    check!(first.values["SdkOperation"] == "First");
    check!(first.metrics["SdkRetryCount"] == 1);
    check!(second.values["SdkOperation"] == "Second");
    check!(second.metrics["SdkRetryCount"] == 2);
    check!(MetricsPool::current().is_none());
}
