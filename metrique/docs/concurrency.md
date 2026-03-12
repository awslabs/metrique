# Concurrency

This module covers patterns for recording metrics across concurrent and
asynchronous operations: flush guards, slots, atomics, and shared handles.

| Primitive | Use case | Works with `Arc`? | Zero-cost? | Example |
|-----------|----------|-------------------|------------|---------|
| [`FlushGuard`](crate::FlushGuard) / [`ForceFlushGuard`](crate::ForceFlushGuard) | Delay emission until background work completes | N/A (type-erased) | Yes | [unit-of-work-fanout] |
| [`Slot`](crate::Slot) | Collect a value from exactly one sub-task | No (oneshot channel) | No (channel overhead) | [Slot example below](#using-slots-to-collect-values-from-tasks) |
| [`Counter`](crate::Counter) / atomics | Fan out to many tasks that increment shared counters | Yes | Yes (atomic ops) | [unit-of-work-fanout] |
| [`Handle`](crate::AppendAndCloseOnDrop::handle) | Share the full metric entry across tasks via `Arc` | Yes (is an `Arc`) | No (Arc overhead) | [Atomics example below](#using-atomics) |

## Metrics with complex lifetimes

Sometimes, managing metrics with a simple ownership and mutable reference pattern does not work well:

```rust,ignore
// Simple case: one owner, one scope, works fine.
async fn handle_request(metrics: &mut RequestMetrics) {
    metrics.duck_count = count_ducks().await;
    // metrics emitted when caller drops the guard
}

// Complex case: multiple tasks need to contribute to the same metric entry.
async fn handle_request_fanout(metrics: &mut RequestMetrics) {
    // Can't move `metrics` into multiple spawned tasks...
    // See the patterns below for solutions.
}
```

The `metrique` crate provides some tools to help more complex situations.

### Controlling the point of metric emission

Sometimes, your code does not have a single exit point at which you want to report your metrics. Maybe
your operation spawns some post-processing tasks, and you want your metric entry to include information
from all of them.

You don't want to wrap your parent metric in an `Arc`, as that will prevent you from having mutable access
to metric fields, but you still want to delay metric emission.

To allow for that, the [`AppendAndCloseOnDrop`](crate::AppendAndCloseOnDrop) guard (which is what the `<MetricName>Guard` aliases point to)
has [`flush_guard`](crate::AppendAndCloseOnDrop::flush_guard) and [`force_flush_guard`](crate::AppendAndCloseOnDrop::force_flush_guard) functions. The flush guards are type-erased (they have
types [`FlushGuard`](crate::FlushGuard) and [`ForceFlushGuard`](crate::ForceFlushGuard), which don't mention the type of the metric entry).

```rust,ignore
let mut metrics = RequestMetrics::init("DoSomething");

// FlushGuard delays emission until dropped. It does not carry metric data;
// use a Slot or atomic fields to pass values back from the spawned task.
let guard = metrics.flush_guard();
tokio::task::spawn(async move { do_work(guard).await });

// ForceFlushGuard: metric emits when ANY force guard drops (e.g. a timeout)
let force_guard = metrics.force_flush_guard();
tokio::task::spawn(async move {
    tokio::time::sleep(Duration::from_secs(30)).await;
    drop(force_guard); // forces emission even if other work is pending
});

// Slot with OnParentDrop::Wait: holds a FlushGuard internally.
// When the slot is closed or dropped, the guard is released and metrics flush.
let slot = metrics.child.open(OnParentDrop::Wait(metrics.flush_guard()));
```

The metric will then be emitted when either:

1. The owner handle of the metric and *all* the [`FlushGuard`](crate::FlushGuard)s have been dropped
2. The owner handle of the metric and *any* of the [`ForceFlushGuard`](crate::ForceFlushGuard)s have been dropped.

This makes [`force_flush_guard`](crate::AppendAndCloseOnDrop::force_flush_guard) useful to emit a metric via a timeout even if some
of the downstream tasks have not completed, which is useful since you normally
want metrics even (maybe *especially*) when things are stuck (the downstream tasks
presumably have access to the metric struct via an [`Arc`](#using-atomics)
or [`Slot`](#using-slots-to-collect-values-from-tasks), which if they eventually finish,
will let them safely write a value to the now-dead metric).

See the examples below to see how the flush guards are used.

### Using `Slot`s to collect values from tasks

In some cases, you might want a sub-task (potentially a Tokio task, but maybe just a sub-component of your code)
to be able to add some metric fields to your metric entry, but without forcing an ownership relationship.

In that case, you can use [`Slot`](crate::Slot), which creates a oneshot channel, over which the value of the metric can be sent.

Note that [`Slot`](crate::Slot) does not delay the parent metric entry's emission. If the parent is emitted before the slot is filled, the slot's metrics are skipped. To avoid this, either wait for the subtask to complete, call [`Slot::wait_for_data`](crate::Slot::wait_for_data), or use [`OnParentDrop::Wait`](crate::OnParentDrop::Wait) to hold a [`FlushGuard`](crate::FlushGuard) until the slot is closed.

```rust
use metrique::writer::GlobalEntrySink;
use metrique::unit_of_work::metrics;
use metrique::{ServiceMetrics, SlotGuard, Slot, OnParentDrop};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,

    // When using a nested field, you must explicitly flatten the fields into the root
    // metric. The slot is closed on drop, which collects results.
    #[metrics(flatten)]
    downstream_operation: Slot<DownstreamMetrics>
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            operation,
            downstream_operation: Default::default()
        }.append_on_drop(ServiceMetrics::sink())
    }
}

// sub-fields can also be declared with `#[metrics]`
#[metrics(subfield)]
#[derive(Default)]
struct DownstreamMetrics {
    number_of_ducks: usize
}

async fn handle_request_discard() {
    let mut metrics = RequestMetrics::init("DoSomething");
    let downstream_metrics = metrics.downstream_operation.open(OnParentDrop::Discard).unwrap();

    // NOTE: if `downstream_metrics` is not dropped before `metrics` (the parent object),
    // no data associated with `downstream_metrics` will be emitted
    tokio::task::spawn(async move {
        call_downstream_service(downstream_metrics)
    });

    // If you want to ensure you don't drop data from a slot if background is still in-flight, you can wait explicitly:
    metrics.downstream_operation.wait_for_data().await;
}

async fn handle_request_on_parent_wait() {
    let mut metrics = RequestMetrics::init("DoSomething");
    let guard = metrics.flush_guard();
    let downstream_metrics = metrics.downstream_operation.open(OnParentDrop::Wait(guard)).unwrap();

    // NOTE: if `downstream_metrics` is not dropped before `metrics` (the parent object),
    // no data associated with `downstream_metrics` will be emitted
    tokio::task::spawn(async move {
        call_downstream_service(downstream_metrics)
    });

    // The metric will be emitted when the downstream service finishes
}


async fn call_downstream_service(mut metrics: SlotGuard<DownstreamMetrics>) {
    // can mutate the struct directly w/o using atomics.
    metrics.number_of_ducks += 1
}
```

### Using Atomics

You might want to "fan out" work to multiple scopes that are in the background or otherwise operating in parallel. You can
accomplish this by using atomic field types to store the metrics, and fanout-friendly wrapper APIs on your metrics entry.

Anything that implements [`CloseValue`](crate::CloseValue) can be used as a field. `metrique` provides a number of basic primitives such as [`Counter`](crate::Counter), a thin wrapper around `AtomicU64`. Most `std::sync::atomic` types also implement [`CloseValueRef`](crate::CloseValueRef) directly. If you need to build your own primitives, implement `CloseValue` for both the owned type and `&T` (see the [`CloseValue`](crate::CloseValue) trait docs). [`CloseValueRef`](crate::CloseValueRef) is then derived automatically. By using primitives that can be mutated through shared references, you make it possible to use [`Handle`](crate::AppendAndCloseOnDrop::handle) or your own `Arc` to share the metrics entry around multiple owners or tasks.

For further usage of atomics for concurrent metric updates, see [the fanout example][unit-of-work-fanout].

```rust
use metrique::writer::GlobalEntrySink;
use metrique::unit_of_work::metrics;
use metrique::{Counter, ServiceMetrics};

use std::sync::Arc;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    number_of_concurrent_ducks: Counter
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            operation,
            number_of_concurrent_ducks: Default::default()
        }.append_on_drop(ServiceMetrics::sink())
    }
}

fn count_concurrent_ducks() {
    let mut metrics = RequestMetrics::init("CountDucks");

    // convenience function to wrap `entry` in an `Arc`. This makes a cloneable metrics handle.
    let handle = metrics.handle();
    for i in 0..10 {
        let handle = handle.clone();
        std::thread::spawn(move || {
            handle.number_of_concurrent_ducks.add(i);
        });
    }
    // Each handle is keeping the metric entry alive!
    // The metric will not be flushed until all handles are dropped!
}
```

<!-- TODO: add an API to spawn a task that will force-flush the entry after a timeout. -->

[unit-of-work-fanout]: https://github.com/awslabs/metrique/blob/main/metrique/examples/unit-of-work-fanout.rs
