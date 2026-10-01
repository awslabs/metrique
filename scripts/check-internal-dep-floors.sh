#!/bin/bash
set -euo pipefail

# Check that every published crate requires the current version of each
# workspace sibling it depends on, e.g. `metrique-core = "0.1.23"` and not
# `"0.1"`.
#
# Inside the workspace, siblings are path dependencies, so builds always use the
# local checkout and a loose requirement goes unnoticed. Once published, a loose
# requirement lets a downstream lockfile pair a new crate with an older sibling
# that lacks the APIs it uses, which resolves and then fails to compile.
#
# release-plz keeps full-version requirements in sync when it bumps a crate,
# but leaves short ones like "0.1" alone, so this check catches new short
# requirements and versions bumped by hand.
#
# Dev-dependencies are skipped: they don't affect downstream builds.
# Unpublished crates (`publish = false`) are skipped.

violations=$(cargo metadata --no-deps --format-version 1 | jq -r '
    (.packages | map({key: .name, value: .version}) | from_entries) as $versions
    | .packages[]
    | select(.publish != [])
    | . as $pkg
    | .dependencies[]
    | select(.kind != "dev")
    | select($versions[.name] != null)
    | select(.req != "^" + $versions[.name])
    | "  \($pkg.name) -> \(.name): requires \"\(.req)\", current version is \($versions[.name])"
')

if [[ -n "$violations" ]]; then
    echo "Internal dependency requirements must match the sibling's current version:"
    echo "$violations"
    echo
    echo "Set the requirement in [workspace.dependencies] in Cargo.toml (or the crate's"
    echo "own Cargo.toml) to the full current version, e.g. version = \"0.1.23\"."
    exit 1
fi

echo "All internal dependency requirements match current versions."
