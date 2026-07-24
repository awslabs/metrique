#!/usr/bin/env bash
set -euo pipefail

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique-writer-core --lib --features _shuttle -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique-writer --lib --features _shuttle -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique-aggregation --lib --features _shuttle -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo test -p metrique --lib --features _shuttle -- shuttle "$@"
