#!/bin/bash
set -euo pipefail

# Check that every published crate requires the current version of each
# workspace sibling it depends on, e.g. `metrique-core = "0.1.23"`.
#
# Inside the workspace, siblings are path dependencies, so builds always use the
# local checkout and a loose requirement goes unnoticed. Once published, a loose
# requirement lets a downstream lockfile pair a new crate with an older sibling
# that lacks the APIs it uses, which resolves and then fails to compile.
#
# Two rules:
#   - Requirements must be full `x.y.z` versions. release-plz rewrites full
#     versions when it bumps a crate but leaves short ones like "0.1" alone.
#   - Requirements must equal the sibling's current version. This catches
#     versions bumped by hand without updating the floor.
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
    | if (.req | test("^\\^[0-9]+\\.[0-9]+\\.[0-9]+$")) then
        "  \($pkg.name) -> \(.name): requires \"\(.req)\" but the current version is \($versions[.name])"
      else
        "  \($pkg.name) -> \(.name): requires \"\(.req)\", which is not a full x.y.z version (current is \($versions[.name]))"
      end
')

if [[ -n "$violations" ]]; then
    echo "Internal dependency requirements must be the sibling's full current version:"
    echo "$violations"
    echo
    echo "Set the requirement in [workspace.dependencies] in Cargo.toml (or the crate's"
    echo "own Cargo.toml) to the full current version, e.g. version = \"0.1.23\"."
    exit 1
fi

echo "All internal dependency requirements match current versions."
