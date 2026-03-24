# metrique-util

Additional utilities for [metrique](https://crates.io/crates/metrique).

## Features

- `state`: Provides [`State<T>`], an atomically swappable shared value with snapshot-on-first-read semantics. Useful for shared runtime state (feature flags, config reloads, routing tables) that should appear on every metric record.
- `tokio-metrics-bridge`: Subscribes [tokio-metrics](https://crates.io/crates/tokio-metrics) runtime snapshots to a global entry sink. The reporter task is automatically aborted when the `AttachHandle` is dropped.

## Usage

```toml
[dependencies]
metrique-util = { version = "0.1", features = ["state"] }
```

See the [metrique documentation](https://docs.rs/metrique) for the full framework.
