#!/usr/bin/env bash
set -euo pipefail

# Each package gets its own --output-dir to avoid overriding the previous package's report.
#
# "pkg:extra rustflags" pairs
packages=(
  "metrique-writer-core:"
  "metrique-writer:"
  "metrique-aggregation:"
  "metrique-util:--cfg tokio_unstable"
  "metrique:"
)

for entry in "${packages[@]}"; do
  pkg="${entry%%:*}"
  extra_rustflags="${entry#*:}"
  RUSTFLAGS="--cfg shuttle $extra_rustflags" \
    cargo llvm-cov --lib --features _shuttle --html --output-dir "target/llvm-cov/$pkg" \
    -p "$pkg" -- shuttle "$@"
  echo "Report: target/llvm-cov/$pkg/html/index.html"
done

open target/llvm-cov/*/html/index.html 2>/dev/null || true
