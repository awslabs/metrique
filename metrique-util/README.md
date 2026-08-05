# metrique-util

Additional utilities for [metrique].

## Features

- `state`: Provides [`State<T>`], an atomically swappable shared value with snapshot-on-first-read semantics. Useful for shared runtime state (feature flags, config reloads, routing tables) that should appear on every metric record.
- `metrics-pool`: Provides [`MetricsPool`], which collects independently-created metrics and flattens them into a parent metric entry. [`with_metrics_pool`] installs a pool while a future is polled.
- `tokio-metrics-bridge`: Subscribes [tokio-metrics] runtime snapshots to a global entry sink. The reporter task is automatically aborted when the `AttachHandle` is dropped.
- `sysinfo-bridge`: Subscribes [sysinfo] system and current-process snapshots to a global entry sink, capturing metrics like CPU usage, disk space, and network rx/tx. The reporter task is automatically aborted when the `AttachHandle` is dropped.
- `pending-sink`: Provides [`pending_sink::new()`], which creates a `(BoxEntrySink, PendingSinkResolver)` pair for deferred sink attachment with bounded buffering. Entries are buffered in a ring buffer until [`PendingSinkResolver::resolve`] drains them into the real sink and switches to direct forwarding. If the resolver is dropped without calling `resolve`, buffered entries are discarded and the sink becomes a no-op.

[tokio-metrics]: https://crates.io/crates/tokio-metrics
[sysinfo]: https://crates.io/crates/sysinfo

## Usage

```toml
[dependencies]
metrique-util = { version = "0.1", features = ["state"] }
```

See the [metrique documentation] for the full framework.

[metrique]: https://crates.io/crates/metrique
[metrique documentation]: https://docs.rs/metrique
[`State<T>`]: https://docs.rs/metrique-util/latest/metrique_util/state/struct.State.html
[`MetricsPool`]: https://docs.rs/metrique-util/latest/metrique_util/struct.MetricsPool.html
[`with_metrics_pool`]: https://docs.rs/metrique-util/latest/metrique_util/fn.with_metrics_pool.html
[`pending_sink::new()`]: https://docs.rs/metrique-util/latest/metrique_util/pending_sink/fn.new.html
[`PendingSinkResolver::resolve`]: https://docs.rs/metrique-util/latest/metrique_util/pending_sink/struct.PendingSinkResolver.html#method.resolve
