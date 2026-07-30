# metrique-shuttle-test

Internal, unpublished (`publish = false`) proc-macro crate. Provides
`#[shuttle_test(..)]`, an attribute that generates the `<name>_pct` /
`<name>_determinism` test pair every [shuttle] test in this workspace needs:

```rust
#[shuttle_test(2_000, 3)]
fn round_trip_no_loss() {
    // ... exercises the code under test, called by both generated tests below
}
```

expands to:

```rust
fn round_trip_no_loss() { /* ... */ }

#[test]
fn round_trip_no_loss_pct() {
    shuttle::check_pct(round_trip_no_loss, 2_000, 3);
}

#[test]
fn round_trip_no_loss_determinism() {
    shuttle::check_uncontrolled_nondeterminism(round_trip_no_loss, 2_000);
}
```

Add `, should_panic = "..."` for a test that expects a panic:

```rust
#[shuttle_test(5_000, 2, should_panic = "some panic message")]
fn concurrent_register_and_drop() { /* ... */ }
```

## Why this exists

Every crate's `mod shuttle_tests` used to hand-write the `_pct`/`_determinism`
pair for each test function -- a dozen-plus times across 5 crates, each a
verbatim repeat of the same three-line pattern. This attribute replaces all of
that boilerplate with one line per test.

## Why it's an unpublished dev-dependency, not a regular one

`#[shuttle_test(..)]` is only ever used inside `#[cfg(test)]` modules --
genuinely test-only, never part of any consumer's compiled library output.
That's what lets this crate stay `publish = false`: a *regular* dependency of
a published crate must itself be published (resolvable from crates.io), but
a *dev*-dependency has no such requirement, since it's never linked when the
depending crate is built as someone else's dependency. See
`docs/shuttle-primitives-consolidation-investigation.md` at the workspace
root for the fuller writeup (including why the shuttle-side primitive shims
like `ArrayQueue`/`Parker`/`OnceSlot` couldn't take the same path and live in
`metrique-writer-core` instead).

## Why not `paste`

An earlier draft used the [`paste`] crate to derive `_pct`/`_determinism` via
token-pasting inside a `macro_rules!`. Rejected: `paste` was archived by its
maintainer in 2024. This attribute macro sidesteps the whole question --
`syn`/`quote` parse the function name directly, no identifier-pasting crate
needed.

[shuttle]: https://github.com/awslabs/shuttle
[`paste`]: https://crates.io/crates/paste
