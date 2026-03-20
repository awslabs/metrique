# metrique-util

Additional utilities for [metrique].

## Features

- `state`: Provides [`State<T>`], an atomically swappable shared value with snapshot-on-first-read semantics. Useful for shared runtime state (feature flags, config reloads, routing tables) that should appear on every metric record.

## Usage

```toml
[dependencies]
metrique-util = { version = "0.1", features = ["state"] }
```

See the [metrique documentation] for the full framework.

[metrique]: https://crates.io/crates/metrique
[metrique documentation]: https://docs.rs/metrique
[`State<T>`]: https://docs.rs/metrique-util/latest/metrique_util/state/struct.State.html
