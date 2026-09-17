// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique::writer::core::Descriptors;
use metrique::writer::test_util::{test_metric, to_test_entry};
use metrique::writer::{Entry, EntryWriter};
use metrique::{CloseValue, InflectableEntry, NameStyle, PascalCase};
use metrique_util::{MetricsPool, MetricsPoolHandle, propagate_current, with_metrics_pool};

#[metrics]
#[derive(Default)]
struct SdkInvocationMetrics {
    operation: &'static str,
    retry_count: u64,
}

// Asymmetric metric types for the frankenentry test. `FailedAttemptMetrics`
// writes an `error` field that `SucceededAttemptMetrics` does not, but both
// share the `latency` field, so under a common prefix a later succeeded attempt
// collides with an earlier failed attempt.
#[metrics(rename_all = "PascalCase")]
struct FailedAttemptMetrics {
    latency: u64,
    error: bool,
}

#[metrics(rename_all = "PascalCase")]
struct SucceededAttemptMetrics {
    latency: u64,
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
        .with_prefix(["sdk", "cloudwatch_logs"])
        .append(SdkInvocationMetrics {
            operation: "Identity",
            retry_count: 1,
        });
    let identity = test_metric(identity);
    check!(identity.values["sdk_cloudwatch_logs_operation"] == "Identity");
    check!(identity.metrics["sdk_cloudwatch_logs_retry_count"] == 1);

    let snake = SnakeRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    snake
        .metrics_pool
        .handle()
        .with_prefix(["sdk", "cloudwatch_logs"])
        .append(SdkInvocationMetrics {
            operation: "Snake",
            retry_count: 2,
        });
    let snake = test_metric(snake);
    check!(snake.values["sdk_cloudwatch_logs_operation"] == "Snake");
    check!(snake.metrics["sdk_cloudwatch_logs_retry_count"] == 2);

    let kebab = KebabRequestMetrics {
        metrics_pool: MetricsPool::new(),
    };
    kebab
        .metrics_pool
        .handle()
        .with_prefix(["sdk", "cloudwatch_logs"])
        .append(SdkInvocationMetrics {
            operation: "Kebab",
            retry_count: 3,
        });
    let kebab = test_metric(kebab);
    check!(kebab.values["sdk-cloudwatch-logs-operation"] == "Kebab");
    check!(kebab.metrics["sdk-cloudwatch-logs-retry-count"] == 3);
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

#[test]
fn superseded_child_is_dropped_wholesale_no_frankenentry() {
    // A failed first attempt records an `Error` field and a `Latency`. A
    // succeeded retry, under the same prefix, records only `Latency`. With
    // per-field last-wins the stray `Error` from attempt one would survive
    // (attempt two never overwrites it), producing a frankenentry. Per-child
    // resolution drops the whole first attempt, so `Error` must NOT survive.
    let pool = MetricsPool::new();
    let handle = pool.handle().with_prefix(["sdk"]);
    handle.append(FailedAttemptMetrics {
        latency: 500,
        error: true,
    });
    handle.append(SucceededAttemptMetrics { latency: 40 });

    let closed = pool.close();
    // A collision occurred, so descriptors are unavailable.
    check!(matches!(
        InflectableEntry::<PascalCase>::descriptors(&closed),
        Descriptors::Unavailable
    ));

    let entry = to_test_entry(PascalEntry(closed));
    // The succeeded attempt wins.
    check!(entry.metrics["SdkLatency"] == 40);
    // The stray error field from the dropped attempt must not leak through.
    check!(!entry.metrics.contains_key("SdkError"));
}

#[test]
fn unrelated_children_with_distinct_prefixes_are_all_kept() {
    // Two unrelated producers share a bare `Latency` field name but use
    // distinct prefixes, so their fully-qualified names never collide and
    // neither child is dropped.
    let pool = MetricsPool::new();
    pool.handle()
        .with_prefix(["s3"])
        .append(SucceededAttemptMetrics { latency: 120 });
    pool.handle()
        .with_prefix(["ddb"])
        .append(SucceededAttemptMetrics { latency: 8 });

    let closed = pool.close();
    // No collision: descriptors remain available.
    check!(!matches!(
        InflectableEntry::<PascalCase>::descriptors(&closed),
        Descriptors::Unavailable
    ));

    let entry = to_test_entry(PascalEntry(closed));
    check!(entry.metrics["S3Latency"] == 120);
    check!(entry.metrics["DdbLatency"] == 8);
}

#[test]
fn three_chained_attempts_keep_only_the_last() {
    // Transitive supersession: each attempt collides with the previous one, so
    // only the final attempt survives.
    let pool = MetricsPool::new();
    let handle = pool.handle().with_prefix(["sdk"]);
    handle.append(SucceededAttemptMetrics { latency: 500 });
    handle.append(SucceededAttemptMetrics { latency: 300 });
    handle.append(SucceededAttemptMetrics { latency: 40 });

    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(entry.metrics["SdkLatency"] == 40);
    // A single value, not an accumulated distribution of all three attempts.
    check!(entry.metrics["SdkLatency"].distribution.len() == 1);
}

#[test]
fn superseded_child_drops_its_non_overlapping_fields_too() {
    // Documents the per-child tradeoff: dropping a child is atomic, so fields
    // that only the dropped child wrote are removed even though no later child
    // supplied a replacement. Here both failed attempts are superseded, so
    // `Error` is absent from the emitted entry rather than retaining a stale
    // value from an earlier attempt.
    let pool = MetricsPool::new();
    let handle = pool.handle().with_prefix(["sdk"]);
    handle.append(FailedAttemptMetrics {
        latency: 500,
        error: true,
    });
    handle.append(FailedAttemptMetrics {
        latency: 300,
        error: false,
    });
    handle.append(SucceededAttemptMetrics { latency: 40 });

    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(entry.metrics["SdkLatency"] == 40);
    check!(!entry.metrics.contains_key("SdkError"));
}

/// A child that writes the same field name twice in a single write pass.
struct DuplicateFieldMetrics;

impl<NS: NameStyle> InflectableEntry<NS> for DuplicateFieldMetrics {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Dup", &1u64);
        writer.value("Dup", &2u64);
    }
}

impl CloseValue for DuplicateFieldMetrics {
    type Closed = DuplicateFieldMetrics;
    fn close(self) -> Self::Closed {
        self
    }
}

/// Records every `value()` call that reaches the downstream writer, plus the
/// entry-level writes a pooled child must never make.
#[derive(Default)]
struct RecordingWriter {
    calls: Vec<String>,
    /// Timestamps forwarded to the parent. A pooled child must contribute none.
    timestamps: Vec<std::time::SystemTime>,
    /// Format configs forwarded to the parent. A pooled child must contribute none.
    configs: usize,
}

impl<'a> EntryWriter<'a> for RecordingWriter {
    fn timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.timestamps.push(timestamp);
    }

    fn value(
        &mut self,
        name: impl Into<std::borrow::Cow<'a, str>>,
        _value: &(impl metrique::writer::Value + ?Sized),
    ) {
        self.calls.push(name.into().into_owned());
    }

    fn config(&mut self, _config: &'a dyn metrique::writer::EntryConfig) {
        self.configs += 1;
    }
}

#[test]
fn duplicate_field_names_within_one_child_are_deduplicated() {
    // Per-child collision handling must not lose per-field dedup *inside* a
    // child. This is not cosmetic: the EMF formatter rejects an entry that
    // writes the same field name twice with
    // `Validation(["for `Dup`: duplicate field"])` and emits no output at all,
    // so a double-write would drop the entire metric record.
    let pool = MetricsPool::new();
    pool.handle().append(DuplicateFieldMetrics);

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
    check!(writer.calls == ["Dup"]);
}

#[test]
fn the_last_write_of_a_repeated_name_wins() {
    let pool = MetricsPool::new();
    pool.handle().append(DuplicateFieldMetrics);

    // `DuplicateFieldMetrics` writes Dup=1 then Dup=2.
    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(entry.metrics["Dup"] == 2);
}

#[test]
fn duplicate_field_names_survive_a_cross_child_drop() {
    // Two children that each duplicate a name and also collide with each other:
    // the earlier child is dropped, and the survivor still emits its name once.
    // Dropping a child must not disturb the surviving child's per-field dedup.
    let pool = MetricsPool::new();
    pool.handle().append(DuplicateFieldMetrics);
    pool.handle().append(DuplicateFieldMetrics);

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
    check!(writer.calls == ["Dup"]);
}

/// A child that writes the same fields in a different order each time it is
/// written, simulating interior mutability (for example, a child backed by a
/// container with unstable iteration order).
struct ReorderingChild {
    flipped: AtomicBool,
}

impl<NS: NameStyle> InflectableEntry<NS> for ReorderingChild {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        if self.flipped.fetch_xor(true, Ordering::SeqCst) {
            writer.value("Bravo", &2u64);
            writer.value("Alpha", &1u64);
        } else {
            writer.value("Alpha", &1u64);
            writer.value("Bravo", &2u64);
        }
    }
}

impl CloseValue for ReorderingChild {
    type Closed = ReorderingChild;
    fn close(self) -> Self::Closed {
        self
    }
}

/// A child that writes a different number of fields each time it is written.
#[cfg(debug_assertions)]
struct VaryingCountChild {
    writes: AtomicUsize,
}

#[cfg(debug_assertions)]
impl<NS: NameStyle> InflectableEntry<NS> for VaryingCountChild {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Alpha", &1u64);
        if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
            writer.value("Extra", &9u64);
        }
    }
}

#[cfg(debug_assertions)]
impl CloseValue for VaryingCountChild {
    type Closed = VaryingCountChild;
    fn close(self) -> Self::Closed {
        self
    }
}

// Field order is not part of the pool's multi-pass invariant. Only the multiset
// of field names must remain stable.
#[test]
fn reordering_child_is_emitted_faithfully() {
    let pool = MetricsPool::new();
    pool.handle().append(ReorderingChild {
        flipped: AtomicBool::new(false),
    });

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
    // The scan pass saw [Alpha, Bravo]; the emit pass writes the flipped order,
    // and both fields still reach the writer under their own names.
    check!(writer.calls == ["Bravo", "Alpha"]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "MetricsPool child changed its field names between write passes")]
fn varying_field_presence_is_rejected() {
    let pool = MetricsPool::new();
    pool.handle().append(VaryingCountChild {
        writes: AtomicUsize::new(0),
    });

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
}

/// A child that writes a repeated field name a different number of times on each
/// write, so the count taken just before emitting does not match what it emits.
#[cfg(debug_assertions)]
struct VaryingRepeatChild {
    writes: AtomicUsize,
}

#[cfg(debug_assertions)]
impl<NS: NameStyle> InflectableEntry<NS> for VaryingRepeatChild {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Dup", &1u64);
        if self.writes.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
            writer.value("Dup", &2u64);
        }
    }
}

#[cfg(debug_assertions)]
impl CloseValue for VaryingRepeatChild {
    type Closed = VaryingRepeatChild;
    fn close(self) -> Self::Closed {
        self
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "MetricsPool child changed its field names between write passes")]
fn varying_field_repetition_is_rejected() {
    let pool = MetricsPool::new();
    pool.handle().append(VaryingRepeatChild {
        writes: AtomicUsize::new(0),
    });

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
}

/// A child that counts how many times the pool writes it, through a counter the
/// test retains after the child is moved into the pool.
struct CountingChild {
    writes: Arc<AtomicUsize>,
}

impl<NS: NameStyle> InflectableEntry<NS> for CountingChild {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        self.writes.fetch_add(1, Ordering::SeqCst);
        writer.value("Alpha", &1u64);
    }
}

impl CloseValue for CountingChild {
    type Closed = CountingChild;
    fn close(self) -> Self::Closed {
        self
    }
}

#[test]
fn a_repeating_child_does_not_slow_down_its_siblings() {
    // The extra counting write is per child, not per pool: a child that writes
    // each name once is emitted directly even when a sibling repeats a name.
    let writes = Arc::new(AtomicUsize::new(0));
    let pool = MetricsPool::new();
    pool.handle().append(CountingChild {
        writes: Arc::clone(&writes),
    });
    pool.handle().append(DuplicateFieldMetrics);
    let closed = pool.close();

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&closed, &mut writer);

    check!(writer.calls == ["Alpha", "Dup"]);
    // Once for the collision scan, once to emit — no counting pass.
    check!(writes.load(Ordering::SeqCst) == 2);
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

/// A child that writes exactly the `(name, value)` pairs it is given.
struct ExactFields(&'static [(&'static str, u64)]);

impl<NS: NameStyle> InflectableEntry<NS> for ExactFields {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        for (name, value) in self.0 {
            writer.value(*name, value);
        }
    }
}

impl CloseValue for ExactFields {
    type Closed = ExactFields;
    fn close(self) -> Self::Closed {
        self
    }
}

/// A child with one distinct dynamically named field, used to observe which
/// entries survive capacity eviction.
struct NumberedField(u64);

impl<NS: NameStyle> InflectableEntry<NS> for NumberedField {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value(format!("Field{}", self.0), &self.0);
    }
}

impl CloseValue for NumberedField {
    type Closed = NumberedField;

    fn close(self) -> Self::Closed {
        self
    }
}

#[test]
fn default_capacity_retains_the_latest_128_children() {
    let pool = MetricsPool::new();
    let handle = pool.handle();
    check!(pool.capacity() == MetricsPool::DEFAULT_CAPACITY);
    check!(pool.capacity() == 128);

    for index in 0..=MetricsPool::DEFAULT_CAPACITY as u64 {
        handle.append(NumberedField(index));
    }

    check!(pool.overflow_count() == 1);
    let closed = pool.close();
    check!(handle.overflow_count() == 1);

    let entry = to_test_entry(PascalEntry(closed));
    check!(!entry.metrics.contains_key("Field0"));
    check!(entry.metrics["Field1"] == 1);
    check!(entry.metrics["Field128"] == 128);
}

#[test]
fn configured_capacity_drops_the_oldest_child() {
    let pool = MetricsPool::builder().capacity(2).build();
    let handle = pool.handle();
    check!(pool.capacity() == 2);

    handle.append(NumberedField(1));
    handle.append(NumberedField(2));
    handle.append(NumberedField(3));

    check!(pool.overflow_count() == 1);
    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(!entry.metrics.contains_key("Field1"));
    check!(entry.metrics["Field2"] == 2);
    check!(entry.metrics["Field3"] == 3);
}

#[test]
fn concurrent_overflow_count_is_exact() {
    const CAPACITY: usize = 8;
    const WORKERS: usize = 4;
    const ENTRIES_PER_WORKER: usize = 32;

    let pool = MetricsPool::builder().capacity(CAPACITY).build();
    let handle = pool.handle();
    let barrier = Arc::new(std::sync::Barrier::new(WORKERS));
    let threads = (0..WORKERS)
        .map(|worker| {
            let handle = handle.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                for index in 0..ENTRIES_PER_WORKER {
                    handle.append(NumberedField((worker * ENTRIES_PER_WORKER + index) as u64));
                }
            })
        })
        .collect::<Vec<_>>();

    for thread in threads {
        thread.join().unwrap();
    }

    check!(pool.overflow_count() as usize == WORKERS * ENTRIES_PER_WORKER - CAPACITY);
    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(entry.metrics.len() == CAPACITY);
}

/// A child whose destructor appends another child to the same pool.
struct AppendOnDrop {
    handle: MetricsPoolHandle,
    replacement: u64,
}

impl<NS: NameStyle> InflectableEntry<NS> for AppendOnDrop {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("EvictMe", &1u64);
    }
}

impl CloseValue for AppendOnDrop {
    type Closed = AppendOnDrop;

    fn close(self) -> Self::Closed {
        self
    }
}

impl Drop for AppendOnDrop {
    fn drop(&mut self) {
        self.handle.append(NumberedField(self.replacement));
    }
}

#[test]
fn evicted_child_is_dropped_after_releasing_the_pool_lock() {
    let pool = MetricsPool::builder().capacity(1).build();
    let handle = pool.handle();
    handle.append(AppendOnDrop {
        handle: handle.clone(),
        replacement: 3,
    });

    // Evicting AppendOnDrop runs its destructor, which re-enters the pool and
    // evicts Field2. This would deadlock if the first eviction were dropped
    // while the collector mutex was held.
    handle.append(NumberedField(2));

    check!(pool.overflow_count() == 2);
    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(!entry.metrics.contains_key("EvictMe"));
    check!(!entry.metrics.contains_key("Field2"));
    check!(entry.metrics["Field3"] == 3);
}

#[test]
#[should_panic(expected = "MetricsPool capacity must be greater than zero")]
fn zero_capacity_is_rejected() {
    let _ = MetricsPool::builder().capacity(0);
}

/// A closed pool holding one [`ExactFields`] child per element of `children`.
fn exact_field_pool(
    children: &[&'static [(&'static str, u64)]],
) -> impl InflectableEntry<PascalCase> {
    let pool = MetricsPool::new();
    for fields in children {
        pool.handle().append(ExactFields(fields));
    }
    pool.close()
}

#[test]
fn transitive_supersession_keeps_the_outer_children_of_the_chain() {
    // child0 {Latency}, child1 {Latency, Error}, child2 {Error}.
    //
    // Survivors are chosen by walking children from last to first and keeping a
    // child only if none of its names are already claimed: child2 is kept, child1
    // collides with it on `Error` and is dropped, and child0's `Latency` is
    // therefore unclaimed and kept. A forward pass instead dropped child0 because
    // of child1 and then dropped child1 too, emitting only `Error` and losing
    // `Latency` entirely.
    let closed = exact_field_pool(&[
        &[("Latency", 10)],
        &[("Latency", 20), ("Error", 21)],
        &[("Error", 32)],
    ]);

    // Both names survive, each exactly once: a duplicate would fail EMF.
    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&closed, &mut writer);
    check!(writer.calls == ["Latency", "Error"]);

    // Each carries the value of the child that was kept, not child1's, which
    // would be a frankenentry.
    let entry = to_test_entry(PascalEntry(closed));
    check!(entry.metrics["Latency"] == 10);
    check!(entry.metrics["Error"] == 32);
}

#[test]
fn a_child_superseded_by_a_dropped_child_is_still_kept() {
    // The same rule with the chain one link shorter: child1 drops out, so child0
    // is not superseded by anything that survives.
    let closed = exact_field_pool(&[
        &[("Alpha", 1)],
        &[("Alpha", 2), ("Bravo", 2)],
        &[("Bravo", 3), ("Charlie", 3)],
    ]);

    let entry = to_test_entry(PascalEntry(closed));
    check!(entry.metrics["Alpha"] == 1);
    check!(entry.metrics["Bravo"] == 3);
    check!(entry.metrics["Charlie"] == 3);
}

#[derive(Debug)]
struct TestConfig;
impl metrique::writer::EntryConfig for TestConfig {}
static TEST_CONFIG: TestConfig = TestConfig;

/// A child that writes everything a standalone entry writes: a timestamp, a
/// format config, and a field.
struct EntryLevelChild;

impl<NS: NameStyle> InflectableEntry<NS> for EntryLevelChild {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(std::time::UNIX_EPOCH);
        writer.config(&TEST_CONFIG);
        writer.value("ChildField", &7u64);
    }
}

impl CloseValue for EntryLevelChild {
    type Closed = EntryLevelChild;
    fn close(self) -> Self::Closed {
        self
    }
}

#[test]
fn pooled_child_entry_metadata_is_suppressed_by_default() {
    for (prefix, expected) in [(None, "ChildField"), (Some("sdk"), "SdkChildField")] {
        let pool = MetricsPool::new();
        let handle = match prefix {
            Some(prefix) => pool.handle().with_prefix([prefix]),
            None => pool.handle(),
        };
        handle.append(EntryLevelChild);

        let mut writer = RecordingWriter::default();
        InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
        check!(writer.timestamps.is_empty());
        check!(writer.configs == 0);
        check!(writer.calls == [expected]);
    }
}

#[test]
fn pooled_child_entry_metadata_can_be_forwarded_explicitly() {
    let pool = MetricsPool::new();
    pool.handle()
        .with_prefix(["sdk"])
        .forward_entry_metadata()
        .append(EntryLevelChild);

    let mut writer = RecordingWriter::default();
    InflectableEntry::<PascalCase>::write(&pool.close(), &mut writer);
    check!(writer.timestamps == [std::time::UNIX_EPOCH]);
    check!(writer.configs == 1);
    check!(writer.calls == ["SdkChildField"]);
}

/// A child that describes its fields *and* writes them twice.
struct DescribedRepeatingChild(SdkInvocationMetricsEntry);

impl<NS: NameStyle> InflectableEntry<NS> for DescribedRepeatingChild {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        InflectableEntry::<NS>::write(&self.0, writer);
        InflectableEntry::<NS>::write(&self.0, writer);
    }

    fn descriptors(&self) -> Descriptors<'_> {
        InflectableEntry::<NS>::descriptors(&self.0)
    }
}

impl CloseValue for DescribedRepeatingChild {
    type Closed = DescribedRepeatingChild;
    fn close(self) -> Self::Closed {
        self
    }
}

#[test]
fn descriptors_are_unavailable_when_a_child_repeats_a_name() {
    // A repeated name is emitted once, so the child's descriptors over-promise:
    // declining to describe is the only honest answer. Both children describe
    // themselves, so the aggregate would otherwise be available.
    let pool = MetricsPool::new();
    pool.handle()
        .with_prefix(["a"])
        .append(DescribedRepeatingChild(
            SdkInvocationMetrics {
                operation: "PutLogEvents",
                retry_count: 1,
            }
            .close(),
        ));
    pool.handle()
        .with_prefix(["b"])
        .append(SdkInvocationMetrics {
            operation: "PutLogEvents",
            retry_count: 2,
        });
    let closed = pool.close();

    check!(matches!(
        InflectableEntry::<PascalCase>::descriptors(&closed),
        Descriptors::Unavailable
    ));
    // The values still arrive, with the repeated names emitted exactly once.
    let entry = to_test_entry(PascalEntry(closed));
    check!(entry.metrics["ARetryCount"] == 1);
    check!(entry.metrics["BRetryCount"] == 2);
}

#[tokio::test]
async fn nested_scopes_restore_the_outer_pool() {
    let outer = MetricsPool::new();
    let inner = MetricsPool::new();

    outer
        .handle()
        .scope(async {
            MetricsPool::current()
                .expect("outer pool is installed")
                .append(ExactFields(&[("OuterBefore", 1)]));

            inner
                .handle()
                .scope(async {
                    MetricsPool::current()
                        .expect("inner pool is installed")
                        .append(ExactFields(&[("Inner", 2)]));
                })
                .await;

            MetricsPool::current()
                .expect("outer pool is restored")
                .append(ExactFields(&[("OuterAfter", 3)]));
        })
        .await;

    let outer_entry = to_test_entry(PascalEntry(outer.close()));
    check!(outer_entry.metrics["OuterBefore"] == 1);
    check!(outer_entry.metrics["OuterAfter"] == 3);
    check!(!outer_entry.metrics.contains_key("Inner"));

    let inner_entry = to_test_entry(PascalEntry(inner.close()));
    check!(inner_entry.metrics["Inner"] == 2);
    check!(!inner_entry.metrics.contains_key("OuterBefore"));
    check!(!inner_entry.metrics.contains_key("OuterAfter"));
}

#[tokio::test]
async fn detached_tasks_do_not_inherit_the_current_pool() {
    let pool = MetricsPool::new();

    let (task_has_no_pool, blocking_task_has_no_pool) = pool
        .handle()
        .scope(async {
            let task = tokio::spawn(async { MetricsPool::current().is_none() });
            let blocking_task = tokio::task::spawn_blocking(|| MetricsPool::current().is_none());
            (task.await.unwrap(), blocking_task.await.unwrap())
        })
        .await;

    check!(task_has_no_pool);
    check!(blocking_task_has_no_pool);
}

#[tokio::test]
async fn propagate_current_carries_the_pool_into_a_spawned_task() {
    let pool = MetricsPool::new();

    pool.handle()
        .scope(async {
            tokio::spawn(propagate_current(async {
                tokio::task::yield_now().await;
                MetricsPool::current()
                    .expect("pool is propagated into the spawned task")
                    .append(ExactFields(&[("Spawned", 1)]));
            }))
            .await
            .unwrap();
        })
        .await;

    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(entry.metrics["Spawned"] == 1);
}

#[tokio::test]
async fn propagate_current_captures_when_the_wrapper_is_created() {
    let pool = MetricsPool::new();
    let mut propagated = None;
    pool.handle()
        .scope(async {
            propagated = Some(propagate_current(async {
                MetricsPool::current()
                    .expect("the construction-time pool is restored")
                    .append(ExactFields(&[("Captured", 1)]));
            }));
        })
        .await;

    check!(MetricsPool::current().is_none());
    propagated.expect("wrapper was created").await;

    let entry = to_test_entry(PascalEntry(pool.close()));
    check!(entry.metrics["Captured"] == 1);
}

#[tokio::test]
async fn propagate_current_without_a_pool_polls_the_future_unchanged() {
    check!(MetricsPool::current().is_none());
    let value = propagate_current(async {
        check!(MetricsPool::current().is_none());
        42
    })
    .await;

    check!(value == 42);
}

#[test]
fn appends_after_parent_close_are_discarded() {
    let pool = MetricsPool::new();
    let handle = pool.handle();
    handle.append(ExactFields(&[("BeforeClose", 1)]));

    let closed = pool.close();
    handle.append(ExactFields(&[("AfterClose", 2)]));

    let entry = to_test_entry(PascalEntry(closed));
    check!(entry.metrics["BeforeClose"] == 1);
    check!(!entry.metrics.contains_key("AfterClose"));
    check!(handle.overflow_count() == 0);
}
