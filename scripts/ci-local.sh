#!/bin/bash
set -e

echo "=== Running Local CI Checks ==="

# Format check
echo "→ Checking formatting..."
cargo +nightly fmt --all -- --check || { echo "FAILED: cargo +nightly fmt --all -- --check"; exit 1; }

# Clippy
echo "→ Running clippy..."
cargo clippy --workspace --all-features -- -D warnings || { echo "FAILED: cargo clippy --workspace --all-features -- -D warnings"; exit 1; }

# Security audit
echo "→ Running security audit..."
cargo audit || { echo "FAILED: cargo audit"; exit 1; }

# Build and test matrix
for toolchain in 1.89.0 stable nightly; do
    for flags in "--all-features" "--no-default-features"; do
        echo "→ Building with $toolchain $flags..."

        if [ "$toolchain" != "stable" ]; then
            rm -fv Cargo.lock
        fi

        cargo +$toolchain nextest run $flags || { echo "FAILED: cargo +$toolchain test --quiet $flags"; exit 1; }
        cargo +$toolchain test --doc --quiet $flags || { echo "Doctests failed: cargo +$toolchain test --docs --quiet $flags"; exit 1; }

        if [ "$toolchain" == "nightly" ] && [ "$flags" == "--all-features" ]; then
            echo "→ Building docs with nightly..."
            RUSTDOCFLAGS="-D warnings --cfg docsrs" cargo +$toolchain doc --quiet --no-deps $flags -Zunstable-options -Zrustdoc-scrape-examples || { echo "FAILED: doc build"; exit 1; }
        fi
    done
done

echo "✓ All CI checks passed!"
