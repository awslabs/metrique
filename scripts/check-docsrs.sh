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

    # Package without verification — we need to patch Cargo.toml before building.
    # cargo package validates features against crates.io, which fails when
    # workspace siblings introduce new features that haven't been published
    # yet. Fall back to building docs directly from the workspace in that case.
    local pkg_err
    if ! pkg_err=$(cargo package -p "$pkg_name" --allow-dirty --no-verify 2>&1); then
        if echo "$pkg_err" | grep -q "does not have that feature"; then
            echo "  ⚠ cargo package failed (unpublished feature), falling back to workspace build"
            local manifest_dir
            manifest_dir=$(cargo metadata --no-deps --format-version 1 | \
                jq -r ".packages[] | select(.name == \"$pkg_name\") | .manifest_path | rtrimstr(\"/Cargo.toml\")")
            (cd "$manifest_dir" && cargo +nightly docs-rs --target "$TARGET")
            return
        fi
        echo "$pkg_err" >&2
        return 1
    fi

    # Extract the .crate tarball (cargo package --no-verify doesn't extract)
    rm -rf "$pkg_dir"
    tar xzf "target/package/$pkg_name-$pkg_version.crate" -C target/package/

    # Patch the extracted Cargo.toml so workspace siblings resolve locally.
    generate_patch_entries "$pkg_name" >> "$pkg_dir/Cargo.toml"

    (cd "$pkg_dir" && cargo +nightly docs-rs --target "$TARGET")
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
