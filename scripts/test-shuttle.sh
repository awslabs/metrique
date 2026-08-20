#!/usr/bin/env bash
set -euo pipefail

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique-writer-core --lib --features _shuttle -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique-writer --lib --features _shuttle -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique-aggregation --lib --features _shuttle -- shuttle "$@"

# metrique-util's dev-dependency self-reference pulls in tokio-metrics-bridge,
# which needs --cfg tokio_unstable regardless of shuttle.
RUSTFLAGS="--cfg shuttle --cfg tokio_unstable" \
  cargo test -p metrique-util --lib --features _shuttle -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique --lib --features _shuttle -- shuttle "$@"
