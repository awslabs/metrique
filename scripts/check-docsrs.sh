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

# cargo package strips doc-scrape-examples from [[example]] sections, but
# docs.rs reads it from the published Cargo.toml. Restore the flag so the
# packaged build faithfully reproduces docs.rs behavior (examples that
# reference dev-only crates will fail to scrape, just like on docs.rs).
restore_doc_scrape_examples() {
    local source_toml=$1
    local packaged_toml=$2

    # Find example names that have doc-scrape-examples = true in the source.
    # Uses awk to track [[example]] sections: when we see the flag, emit the
    # most recent name.
    local names
    names=$(awk '
        /^\[\[example\]\]/     { name="" }
        /^name *= *"/ { match($0, /"([^"]+)"/, m); name=m[1] }
        /^doc-scrape-examples *= *true/ { if (name != "") print name }
    ' "$source_toml")

    local ex
    for ex in $names; do
        # Only inject if not already present (some cargo versions preserve it).
        if ! awk -v name="$ex" '
            /^\[\[example\]\]/ { if (!found_scrape) found_name=0 }
            /^name *= *"/ { match($0, /"([^"]+)"/, m); if (m[1]==name) found_name=1 }
            found_name && /^doc-scrape-examples *= *true/ { found_scrape=1 }
            END { exit !found_scrape }
        ' "$packaged_toml"; then
            sed -i "/^name = \"$ex\"/a doc-scrape-examples = true" "$packaged_toml"
        fi
    done
}

check_package() {
    local pkg_name=$1
    local pkg_version=$2
    local pkg_dir="target/package/$pkg_name-$pkg_version"

    echo "→ Checking docs.rs build for $pkg_name..."

    # cargo package + docs-rs on the packaged crate catches workspace unification bugs
    # and dev-dependency leaks into scraped examples. Falls back to building directly
    # from the workspace only when cargo package itself fails (e.g. unpublished crates
    # or features not yet on crates.io).
    if cargo package -p "$pkg_name" --allow-dirty --no-verify 2>/dev/null; then
        # Extract the .crate tarball (cargo package --no-verify doesn't extract)
        rm -rf "$pkg_dir"
        tar xzf "target/package/$pkg_name-$pkg_version.crate" -C target/package/

        # Patch the extracted Cargo.toml so workspace siblings resolve locally.
        generate_patch_entries "$pkg_name" >> "$pkg_dir/Cargo.toml"

        # Restore doc-scrape-examples = true that cargo package strips.
        local source_toml
        source_toml=$(cargo metadata --no-deps --format-version 1 | \
            jq -r --arg name "$pkg_name" '.packages[] | select(.name == $name) | .manifest_path')
        restore_doc_scrape_examples "$source_toml" "$pkg_dir/Cargo.toml"

        (cd "$pkg_dir" && cargo +nightly docs-rs --target "$TARGET")
        return
    fi

    echo "  ⚠ cargo package failed, falling back to workspace build"
    cargo +nightly docs-rs -p "$pkg_name" --target "$TARGET"
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
