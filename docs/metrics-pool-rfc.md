# RFC: Metrics pools for request-scoped metric collection

> Status: RFC with prototype
>
> Applies to: `metrique`, `metrique-util`

For a summarized list of proposed changes, see the
[Changes checklist](#changes-checklist).

This RFC defines `MetricsPool`, a collector that lets independently-owned
middleware and libraries contribute fields to one parent metrics entry. A pool
can be passed explicitly like a regular sink or discovered through a scoped
current-pool API. The scoped API is intended for synchronous callbacks, such as
AWS SDK interceptors, that execute while a request future is being polled but
cannot accept another application parameter.

## Terminology

- **Parent entry**: The metrics struct that owns a flattened `MetricsPool` and
  is eventually appended to the application's real sink.
- **Child metric**: A metrics struct appended to a pool and flattened into the
  parent when the parent closes.
- **Pool handle**: A cloneable producer, `MetricsPoolHandle`, that appends child
  metrics without exposing previously appended metrics.
- **Current pool**: The pool installed while a scoped future is being polled.
- **Standalone entry**: An entry emitted directly to an application sink when
  no current pool is available.

## The user experience if this RFC is implemented

This section is written as the public documentation users should be able to
read once the API is released.

### Capability matrix

Metric contribution has two independent choices: whether the destination is
strongly or dynamically typed, and how a producer obtains it.

| Destination typing | Explicitly passed | OS thread-local | Async task-local |
| --- | --- | --- | --- |
| Strongly typed | Supported today: pass `&mut RequestMetrics` or a typed child guard | Not supported | Not currently supported; reserve for a future typed task-local API |
| Dynamically typed | This RFC: pass `MetricsPoolHandle` as a regular sink | Deliberately not exposed because async work migrates between threads | This RFC: `MetricsPool::current()` within a poll-scoped future |

Explicit, strongly typed access remains the preferred choice when the producer
and parent metric can share a concrete type. `MetricsPool` is for integration
boundaries where that coupling is unavailable. Its current-pool API has
task-local semantics even though its executor-independent implementation uses
a thread-local only during each future poll.

### Collect child metrics in a parent entry

Enable the `metrics-pool` feature on `metrique-util`:

```toml
[dependencies]
metrique-util = { version = "...", features = ["metrics-pool"] }
```

Add a flattened `MetricsPool` to the metrics entry that represents the unit of
work. Normal flatten-site prefixes are supported:

```rust,ignore
use metrique::unit_of_work::metrics;
use metrique_util::MetricsPool;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,

    #[metrics(flatten, prefix = "child_")]
    child_metrics: MetricsPool,
}
```

Code with direct access to the parent can obtain a cloneable handle for a
suboperation:

```rust,ignore
let request = RequestMetrics {
    operation: "PutObject",
    child_metrics: MetricsPool::new(),
};

let suboperation_pool = request
    .child_metrics
    .handle()
    .with_prefix(["sdk", "s3"]);

suboperation_pool.append(SuboperationMetrics {
    operation: "PutObject",
    retry_count: 2,
});

// Closing `request` emits one object containing the parent fields followed by
// the contributed suboperation fields shown below.
```

`MetricsPoolHandle` implements the normal entry sink trait. It also has
`append`, `append_on_drop`, and `append_on_drop_default` convenience methods.
The handle can be cloned into request extensions, middleware, or library state.
It cannot inspect or remove entries already contributed by another producer.

When `RequestMetrics` closes, the suboperation fields are part of that same
entry:

```text
Operation = PutObject
ChildSdkS3Operation = PutObject
ChildSdkS3RetryCount = 2
```

`ChildSdkS3` is the composition of the flatten-site prefix `child_` and the
producer prefix `["sdk", "s3"]`. The Rust field name `child_metrics` does not
appear in metric names; flattened field names are controlled only by explicit
prefixes.

### Use a current pool from callbacks

The normal API accepts an owned handle and a future:

```rust,ignore
use metrique_util::with_metrics_pool;

let mut request_metrics = RequestMetrics::new();
let pool = request_metrics.child_metrics.handle();
let request_metrics = with_metrics_pool(pool, async move {
    handle_request(&mut request_metrics).await;
    request_metrics
}).await;
```

Code running synchronously during a poll can discover the installed pool:

```rust,ignore
if let Some(pool) = MetricsPool::current() {
    pool.with_prefix(["sdk", "s3"]).append(suboperation_metrics);
}
```

`MetricsPool::current()` is the only API that discovers scoped state. Methods
such as `with_prefix` operate on an explicit `MetricsPoolHandle`. Library code
should use `current()`'s `Option` to select its standalone fallback.

The scope follows future polling rather than an operating-system thread:

- A future can migrate between executor threads without losing its pool.
- Concurrent futures polled on the same thread do not see each other's pools.
- Nested scopes use the innermost pool.
- A separately spawned task does not inherit the pool. Pass a
  `MetricsPoolHandle` explicitly when detached work should contribute.

This API does not require Tokio and is usable with any executor.

### Crossing a spawn boundary

`current()` returns `None` for two reasons a producer cannot tell apart: no
application ever installed a pool, or one was installed by a caller whose
context this code no longer shares. Nothing in Rust carries a poll-scoped or
task-local value across a spawn, so the pool is genuinely gone by the time
detached code runs and no API can reconstruct which case it is in.

Detached work can carry the current scope by wrapping its future at the spawn
site, while the parent scope is still installed:

```rust,ignore
tokio::spawn(propagate_current(detached_work()));
```

`propagate_current` captures `MetricsPool::current()` synchronously when called
and reinstalls that handle for every poll of the wrapped future. It is
executor-independent and transparent when no pool is installed, so callers can
wrap spawned futures unconditionally. Like any wrapper, it can only propagate
across spawn sites where it is applied; it cannot affect tasks spawned internally
by third-party code.

Callers can instead capture and pass a handle explicitly when detached work owns
an appropriate parameter:

```rust,ignore
let pool = MetricsPool::current();
tokio::spawn(async move { detached_work(pool).await });
```

### Prefixes and casing

Prefixes are paths of static string segments:

```rust,ignore
let pool = pool.with_prefix(["sdk", "cloudwatch_logs", "logging"]);
```

Both prefixes and child field names follow the parent entry's name style:

| Parent style | Result |
| --- | --- |
| identity | `sdk_cloudwatch_logs_logging_retry_count` |
| PascalCase | `SdkCloudwatchLogsLoggingRetryCount` |
| snake_case | `sdk_cloudwatch_logs_logging_retry_count` |
| kebab-case | `sdk-cloudwatch-logs-logging-retry-count` |

The pool also respects prefixes on the flatten site. Use `prefix` for a prefix
that follows the parent casing, or `exact_prefix` to preserve punctuation:

```rust,ignore
#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(flatten, exact_prefix = "sdk.request.")]
    child_metrics: MetricsPool,
}
```

This produces names such as `sdk.request.RetryCount`. A producer can add the
same kind of literal prefix at runtime:

```rust,ignore
pool.with_exact_prefix("sdk.request.").append(metrics);
```

Static prefix segments allow the fully inflected paths to be interned and used
by entry descriptors without leaking caller-owned runtime strings.

### Duplicate field names

Two child metrics can produce the same fully inflected field name. The later
child wins, and the earlier child is dropped in its entirety rather than merged
field by field:

```rust,ignore
let attempt_pool = request.child_metrics.handle().with_prefix(["sdk", "s3"]);

// A first attempt that failed reports its error.
attempt_pool.append(FailedAttemptMetrics { latency: 500, error: true });
// The retry that succeeded reports latency only.
attempt_pool.append(SucceededAttemptMetrics { latency: 40 });
```

Both children write the same name under the `child_` flatten site and the
`["sdk", "s3"]` producer prefix, so only the retry reaches the parent:

```text
ChildSdkS3Latency = 40
```

`ChildSdkS3Error` is absent. Retaining it would pair the failed attempt's error
with the retry's latency and describe an attempt that never occurred, so a
superseded child takes its remaining fields with it. Producers whose fields must
coexist need distinct prefixes.

Survivors are selected by walking children from last to first, keeping a child
only if none of its field names were already claimed by a child that was kept. For
two colliding children this is simply "the later child wins". For longer chains it
also avoids dropping a field that no survivor replaces: if child A writes
`Latency`, child B writes `Latency` and `Error`, and child C writes `Error`, then
C and A are kept and B is dropped, so both names still reach the parent. A forward
"latest writer wins" pass would drop both A and B and lose `Latency` entirely,
even though no surviving child claimed it.

Selecting survivors requires one pass over each child to collect fully-qualified
field-name counts, followed by another pass over surviving children to emit
them. A surviving child that repeats a name gets an additional counting pass so
only its last write is emitted. A closed child must therefore produce the same
multiset of field names on every write. Values and field order may change, but
interior mutability must not change which names are present or how many times a
name is written. Entries generated by `#[metrics]` satisfy this unless they
flatten a custom entry that violates the requirement. Debug builds assert this
invariant; release builds rely on it.

A rate limited warn naming the colliding fields is emitted at most once per
minute. The pool compares only the fields of its own children: a collision with a
regular parent field or with another flattened component is not detected.

### Parent close and late producers

Closing the parent takes all child metrics available at that moment. A handle
may outlive its parent, but appends made after the pool closes are discarded.
Closing does not wait for outstanding handles. A rate limited warn is emitted
at most once per minute in the event this happens.

`append_on_drop` guards therefore need to be dropped before the parent entry
closes. Future work may add an explicit bounded wait policy, but an unbounded
wait is not part of this proposal.

### AWS SDK interceptor pattern

Install the interceptor on each long-lived SDK client configuration. Generated
AWS SDK service builders expose interceptor registration directly; callers do
not need to construct a runtime plugin:

```rust,ignore
let cw_logs_config = aws_sdk_cloudwatchlogs::Config::builder()
    .region(region)
    .credentials_provider(credentials)
    .interceptor(
        AwsSdkMetricsInterceptor::builder("CloudWatchLogs")
            .qualifier("Logging")
            .build(),
    )
    .build();
let client = aws_sdk_cloudwatchlogs::Client::from_conf(cw_logs_config);
```

The builder defaults to "use the current request pool when one exists,
otherwise use the global sink." Applications that expect every invocation to be
request-scoped can opt into `require_current_pool()`. The interceptor chooses its
destination once in `read_before_execution` and stores it in interceptor state:

```rust,ignore
#[derive(Clone, Debug)]
struct CapturedPool(Option<MetricsPoolHandle>);

impl Storable for CapturedPool {
    type Storer = StoreReplace<Self>;
}

fn read_before_execution(
    &self,
    _context: &BeforeSerializationInterceptorContextRef<'_>,
    cfg: &mut ConfigBag,
) -> Result<(), BoxError> {
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
```

`selected_pool` returns the handle supplied to the builder through
`metrics_pool`, or calls `MetricsPool::current()` when no explicit handle was
configured. Under the default policy, a missing current pool selects the
standalone fallback. With `require_current_pool()`, it triggers a debug assertion
so lost scope propagation is caught during development; release builds emit a
rate-limited warning and retain the standalone fallback so metrics and SDK calls
are not lost. The warning can report the static service and qualifier, but cannot
claim that a spawn occurred because no installed pool and a lost scope are
indistinguishable at the discovery point.

Capturing once gives all retries and finalization hooks the destination selected
when the invocation began. `read_after_attempt` can continue emitting the
existing standalone attempt entries. `read_after_execution` sends the final
attempt plus invocation duration and retry count to exactly one destination:

```rust,ignore
let pool = cfg
    .load::<CapturedPool>()
    .and_then(|captured| captured.0.as_ref());

if let Some(pool) = pool {
    pool.append(invocation);
} else {
    drop(append_and_close(
        StandaloneSdkInvocationMetrics {
            timestamp,
            metrics_pool_fallback: true,
            invocation,
        },
        ServiceMetrics::sink(),
    ));
}
```

The pooled payload should contain fields only. Entry-level timestamp and EMF
configuration stay on the standalone wrapper so an SDK child does not replace
or reconfigure the enclosing request entry. Standalone invocation entries also
emit `MetricsPoolFallback = 1`, making fallback volume observable in production.
This records the selected destination, not the reason it was selected.

The strict policy is configured explicitly:

```rust,ignore
let interceptor = AwsSdkMetricsInterceptor::builder("CloudWatchLogs")
    .qualifier("Logging")
    .require_current_pool()
    .build();
```

It is appropriate only when standalone use is unexpected. The default remains
lenient because client-wide interceptors may legitimately serve background work
or applications that do not install a request pool.

When a call site already has a pool handle, it can avoid scoped discovery and
pass the destination through a per-operation interceptor:

```rust,ignore
let pool = request_metrics.child_metrics.handle();
client
    .put_log_events()
    .log_group_name(group)
    .log_stream_name(stream)
    .log_events(event)
    .customize()
    .interceptor(
        AwsSdkMetricsInterceptor::builder("CloudWatchLogs")
            .qualifier("Logging")
            .metrics_pool(pool)
            .build(),
    )
    .send()
    .await?;
```

Client-wide current-pool registration and per-call explicit registration are
alternatives; installing both would run two interceptor instances. Explicit
passing is preferable when the call site owns a handle. Scoped discovery is
for shared clients and middleware that cannot change every SDK call.

## How to actually implement this RFC

### Collector ownership

`MetricsPool` owns an `Arc<MetricsPoolInner>`:

```rust,ignore
struct MetricsPoolInner {
    entries: Mutex<Option<VecDeque<BufferedEntry>>>,
    capacity: usize,
    overflow_count: AtomicU64,
}
```

The default capacity is 128 child entries and can be changed through
`MetricsPool::builder().capacity(...)`. Capacity counts appended children, not
the fields they write. An append to a full pool evicts the oldest child before
retaining the new one. Appends never wait for free capacity and follow the
drop-oldest policy required by `EntrySink`. Each eviction increments
`overflow_count` and produces a rate-limited warning.

Each buffered entry contains the producer's prefixes, metadata policy, and four
type-erased entry adapters:

```rust,ignore
struct BufferedEntry {
    prefixes: PrefixSet,
    forward_entry_metadata: bool,
    entry: StyledEntries,
}

struct StyledEntries {
    identity: BoxEntry,
    pascal: BoxEntry,
    snake: BoxEntry,
    kebab: BoxEntry,
}
```

The `Arc` around the collector allows producer handles to be cloned. An append
closes the child and constructs its `BufferedEntry` before acquiring the
collector mutex. The mutex is held only while pushing that closed, type-erased
entry or taking the deque during parent close; user metric code is never
invoked while the lock is held.

The `Option` is the close marker. `Some(VecDeque<_>)` accepts appends. Closing
takes the deque and leaves `None`; later appends observe `None` and are
discarded. Evicted children and children appended after close are destroyed only
after releasing the collector mutex.

Only `MetricsPool` can take the deque. `MetricsPoolHandle` contains the shared
inner allocation and its prefix set, but no API for reading entries. This gives
the isolation benefit of the channel design discussed in metrique issue #159
without requiring a receiver-drain loop.

### Closing and sink compatibility

`MetricsPool` implements `CloseValue` and closes to `MetricsPoolEntry`.
`MetricsPoolEntry` implements `InflectableEntry<NS>` for every `NameStyle`.
That makes a pool valid as a normal `#[metrics(flatten)]` field.

`MetricsPoolHandle` implements:

```rust,ignore
EntrySink<RootEntry<M>>
```

for closed metrics that support identity, PascalCase, snake_case, and
kebab-case. `RootEntry::into_inner` removes the root selected by
`append_and_close`; the child is re-rooted later using the parent's name style.

### Runtime casing and prefixes

The concrete child type is erased when it enters the heterogeneous pool, but
name style remains a generic parameter until the parent closes. Each buffered
child therefore stores four `BoxEntry` adapters:

- identity
- PascalCase
- snake_case
- kebab-case

The adapters share one `Arc` containing the closed child metric. At write time,
the parent's `NameStyle::DESCRIPTOR_STYLE_INDEX` selects one adapter and its
matching inflected prefix. `PrefixedEntryWriter` prepends the selected prefix to
each owned field name and applies the same prefix to descriptor segments.

`NameStyle` may also carry prefixes introduced by enclosing
`#[metrics(flatten, prefix = ...)]` attributes. The write path recovers that
type-level prefix by inflecting an empty name, then prepends it before the
handle's prefix. Exact prefixes use the same path without inflection. Generated
descriptor code applies its flatten-site prefix modifier, while the pool
applies the handle prefix to each child descriptor.

This is deliberately more expensive than a statically typed flattened field.
The pool is for integration boundaries where heterogeneous metric types are
required.

### Performance benchmark

`metrique-util/benches/metrics_pool.rs` uses Divan to compare a direct,
strongly typed write with the pool's type erasure and replayed write path. It
measures:

- appending uniquely prefixed children, including construction of four
  adapters;
- appending the same children into a plain `Vec<Box<dyn Any>>`, as an
  append-side baseline;
- closing and writing uniquely named children;
- closing and writing colliding children; and
- directly writing the same closed child type without a pool.

Each case runs with 1, 4, and 16 children, covering the expected request-level
pool sizes. Run it with:

```console
cargo bench -p metrique-util --bench metrics_pool --features metrics-pool
```

### Poll-scoped current pool

The current pool is:

```rust,ignore
thread_local! {
    static CURRENT_POOL: RefCell<Vec<MetricsPoolHandle>> = ...;
}
```

`MetricsPoolScope<F>` is a future wrapper. Every call to `poll` pushes its
handle, polls the inner future once, and pops through an RAII guard. The handle
is installed only while user code for that future is executing. The stack
provides nested-scope behavior and cleanup during unwinding.

This uses thread-local storage as an implementation mechanism but provides
task-local behavior because installation is scoped to one future poll. It also
avoids a hard dependency on a particular async runtime.

A scope is therefore lost at every spawn boundary, and lent out by every
`block_on` inside a poll. See [Crossing a spawn
boundary](#crossing-a-spawn-boundary) for what a producer can do about each and
which mitigations are deferred.

### Per-child collision pass

Children retain append order. When a later child emits a fully-inflected field
name an earlier child already emitted, the later child wins and the earlier child
is dropped in its entirety:

```rust,ignore
pool.append(SdkInvocationMetrics {
    operation: "First",
    retry_count: 0,
});
pool.append(SdkInvocationMetrics {
    operation: "Second",
    retry_count: 2,
});
```

The parent contains `Operation = Second` and `RetryCount = 2`. A field that only
the first child wrote is dropped along with it rather than surviving beside the
second child's values.

`MetricsPoolEntry::write` evaluates closed child entries twice, and a child that
emits the same field name more than once a third time. `InflectableEntry` and
`Entry` are expected to be pure, but the trait does not require this. The replay
does not consume or mutate macro-generated children; manual implementations
observe each call.

The first pass writes each child into a `RepeatCount`:

1. Apply the parent name style and each child's prefix.
2. Record, per child, how many times it emitted each fully-inflected field name.
3. Walk children from last to first, keeping a child only if none of its names
   were already claimed by a child that was kept. A dropped child claims nothing,
   so an earlier child writing only its fields still survives.
4. Record a child that emitted one of its own names more than once.

If collisions exist, the write path invokes metrique-writer's cross-thread
`rate_limited!` helper around a `tracing::warn!` containing the colliding field
names. The warning is emitted at most once per minute. Duplicate fields the scan
saw are removed before the parent reaches an output formatter, so strict EMF
validation does not reject an otherwise valid entry.

The second pass skips superseded children. A surviving child is forwarded
straight to the parent's writer, so the common case pays nothing beyond the scan.
A child that emitted one of its own names twice is instead counted into
`RepeatCount` and forwarded through `LastWinsEntryWriter`, which emits only that
name's final write. Iteration still follows original append and field order; only
superseded children and superseded repeats are skipped.

Collision handling assumes each child writes the same field names on both passes,
which children generated by `#[metrics]` do. A child that writes a name the scan
never saw escapes both collision checks, and a duplicate field makes EMF reject the
whole entry.

Descriptors are chained normally when the scan finds nothing to remove. If a
collision or a repeated name removed writes, `descriptors()` returns
`Descriptors::Unavailable`, because the original descriptor sequence contains
fields removed by the emit pass. This trades a descriptor optimization for
correctness only on the error path.

Collision handling only compares child fields inside one pool. A flattened
field cannot observe fields written by other fields in its parent, so the pool
cannot detect a collision with a normal parent field or another flattened
component. Producers must use distinct prefixes to avoid those collisions.

A future collision policy could preserve every child by renaming a group
to `Field.1`, `Field.2`, and `Field.3`. The first pass already provides the
information needed to do this, but the policy needs an explicit constructor
and must make descriptors unavailable whenever numbering occurs. It is not part
of the prototype; dropping superseded children with a rate-limited warning
remains the default.

### Sampling and entry metadata

Pooled children do not contribute sample-group elements. An opaque library or
middleware must not silently change the parent request's sampling policy.
Future tail-sampling support may add an explicit opt-in for known child types.

Timestamp and `EntryConfig` calls are suppressed by default. This keeps an
opaque child from replacing the parent timestamp or adding incompatible format
configuration that invalidates the entire record. A producer that intentionally
needs its child to control entry-level metadata can opt in for entries appended
through a handle by calling `MetricsPoolHandle::forward_entry_metadata()`. The
setting is preserved when the handle is cloned or prefixed. Child sample groups
remain suppressed even when entry metadata forwarding is enabled.

### SDK interceptor adaptation

The interceptor remains registered through each AWS service config
builder's `.interceptor(...)` method and keeps existing attempt behavior:

1. `read_before_execution` stores invocation timing, retry state, and an
   optional captured pool handle selected from either the explicit interceptor
   parameter or `MetricsPool::current()`.
2. `read_before_attempt` increments attempts and starts attempt timing.
3. `read_after_attempt` emits one standalone attempt entry and stores the latest
   attempt in `ConfigBag`.
4. `read_after_execution` builds one field-only invocation summary.
5. If a pool was captured, append the summary with the service and qualifier
   prefix.
6. Otherwise wrap the summary with timestamp and EMF configuration and append
   it to `ServiceMetrics`.

This preserves existing global metrics for background jobs and other callers
outside a request scope while avoiding duplicate invocation entries for scoped
requests.

## Changes checklist

### Prototype

- [x] Add the `metrique-util/metrics-pool` feature.
- [x] Implement `MetricsPool` as a flattened metric field.
- [x] Implement cloneable `MetricsPoolHandle`.
- [x] Implement `EntrySink<RootEntry<M>>` for pool handles.
- [x] Add immediate and append-on-drop convenience methods.
- [x] Add identity, PascalCase, snake_case, and kebab-case propagation.
- [x] Add static multi-segment prefixes.
- [x] Preserve inflected and exact prefixes on the pool's flatten site.
- [x] Add exact producer prefixes for punctuation-delimited names.
- [x] Add poll-scoped current-pool discovery without a Tokio dependency.
- [x] Add `with_metrics_pool`.
- [x] Add executor-independent `propagate_current` for controlled spawn sites.
- [x] Ignore child sample groups.
- [x] Discard appends made after parent close, with a rate-limited warning.
- [x] Bound pools to 128 child entries by default, with configurable capacity,
  oldest-child eviction, an overflow counter, and a rate-limited warning.
- [x] Implement per-child field collision resolution.
- [x] Keep only the last write of a name a single child repeats.
- [x] Add a once-per-minute collision warning.
- [x] Mark descriptors unavailable when collisions or repeated names remove
  fields.
- [x] Add an AWS Smithy interceptor example with client-wide discovery,
  explicit per-call injection, and global fallback.
- [x] Add a Divan benchmark for append and write costs at expected pool sizes,
  with append-side and write-side baselines.
- [x] Test casing, task isolation, per-child collision resolution, repeated
  field names, pooled SDK emission, explicit pool injection, flatten prefixes,
  and global SDK fallback.

### Before stabilization

- [ ] Decide whether `MetricsPool` belongs permanently in `metrique-util` or a
  smaller core crate shared by middleware integrations.
- [ ] Decide whether `metrics-pool` should remain feature-gated.
- [ ] Decide whether prefixes must remain static or descriptors should support
  owned runtime prefixes.
- [x] Suppress child timestamps and `EntryConfig` by default and provide an
  explicit per-handle forwarding opt-in.
- [ ] Decide whether any known child types may opt into parent sampling.
- [ ] Decide whether collision handling must include fields outside the pool.
- [ ] Decide whether to add an opt-in numbered collision policy such as
  `Field.1`, `Field.2`, `Field.3`.
- [ ] Decide whether the pool needs an explicit tombstone mechanism that can
  remove an earlier emitted value.
- [ ] Decide whether a bounded close wait for outstanding child guards is
  needed.
- [x] Add a combinator that carries the current pool across spawns controlled by
  the caller.
- [ ] Add a reusable Tower layer that owns the parent metrics guard and installs
  its pool at the request boundary.
- [ ] Add compile-fail coverage for unsupported child metric types.
- [ ] Add coverage for the scope boundaries: that a spawn loses the pool, and
  that whatever propagation mechanism is chosen carries it.
- [x] Add crate-level public documentation and a changelog entry.
- [ ] Review names for `MetricsPool`, `MetricsPoolHandle`, `with_metrics_pool`,
  `propagate_current`, and `current`.
