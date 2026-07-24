#!/usr/bin/env bash
set -euo pipefail

RUSTFLAGS="--cfg shuttle" \
  cargo llvm-cov --lib --features _shuttle --html -p metrique-writer-core -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo llvm-cov --lib --features _shuttle --html -p metrique-writer -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo llvm-cov --lib --features _shuttle --html -p metrique-aggregation -- shuttle "$@"

RUSTFLAGS="--cfg shuttle" \
  cargo llvm-cov --lib --features _shuttle --html -p metrique -- shuttle "$@"

echo "Report: target/llvm-cov/html/index.html"
open target/llvm-cov/html/index.html 2>/dev/null || true
