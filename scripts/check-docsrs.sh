#!/bin/bash
set -e

# Simulate docs.rs build for metrique packages
# Usage:
#   ./scripts/check-docsrs.sh           # Run on all workspace packages
#   ./scripts/check-docsrs.sh <package> # Run on specific package

# Determine the target to use based on installed nightly targets
TARGET=$(rustup target list --installed --toolchain nightly | head -1)

# Build [patch.crates-io] entries so the packaged crate resolves workspace
# siblings from the local checkout instead of crates.io. Without this, any
# new public API added in a workspace crate that hasn't been published yet
# will fail to resolve.
generate_patch_entries() {
    local pkg_name=$1
    echo '[patch.crates-io]'
    cargo metadata --no-deps --format-version 1 | \
        jq -r --arg skip "$pkg_name" \
        '.packages[] | select(.name != $skip) | "\(.name) = { path = \"\(.manifest_path | rtrimstr("/Cargo.toml"))\" }"'
}

check_package() {
    local pkg_name=$1
    local pkg_version=$2
    local pkg_dir="target/package/$pkg_name-$pkg_version"

    echo "→ Checking docs.rs build for $pkg_name..."

    # Try to package the crate. This can fail when workspace siblings have
    # new features that haven't been published yet (cargo package validates
    # features against crates.io). In that case, fall back to building docs
    # directly from the workspace.
    if cargo package -p "$pkg_name" --allow-dirty --no-verify 2>/dev/null; then
        # Extract the .crate tarball (cargo package --no-verify doesn't extract)
        rm -rf "$pkg_dir"
        tar xzf "target/package/$pkg_name-$pkg_version.crate" -C target/package/

        # Patch the extracted Cargo.toml so workspace siblings resolve locally.
        generate_patch_entries "$pkg_name" >> "$pkg_dir/Cargo.toml"

        (cd "$pkg_dir" && cargo +nightly docs-rs --target "$TARGET")
    else
        echo "  ⚠ cargo package failed (likely unpublished features), building docs from workspace"
        cargo +nightly rustdoc \
            -p "$pkg_name" \
            --all-features \
            --target "$TARGET" \
            -- --cfg docsrs
    fi
}

if [ $# -eq 0 ]; then
    # Run on all workspace packages
    packages=$(cargo metadata --no-deps --format-version 1 | \
        jq -r '.packages[] | "\(.name) \(.version)"')

    while IFS= read -r line; do
        pkg_name=$(echo "$line" | cut -d' ' -f1)
        pkg_version=$(echo "$line" | cut -d' ' -f2)
        check_package "$pkg_name" "$pkg_version"
    done <<< "$packages"
else
    # Run on specific package
    pkg_name=$1
    pkg_version=$(cargo metadata --no-deps --format-version 1 | \
        jq -r ".packages[] | select(.name == \"$pkg_name\") | .version")

    if [ -z "$pkg_version" ]; then
        echo "Error: Package $pkg_name not found in workspace"
        exit 1
    fi

    check_package "$pkg_name" "$pkg_version"
fi

echo "✓ All docs.rs checks passed!"
