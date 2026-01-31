# Metrique Workspace Guidelines

## Testing
- Use `cargo +1.89 nextest run` to run all tests in this workspace
- To run the full suite of tests CI will run, see `scripts/ci-local.sh`.
- If there are mismatches in trybuild or insta snapshots, share the diff for user approval before accepting them
- Before commiting run `cargo fmt` and `cargo clippy`. YOU MUST FIX CLIPPY ERRORS.
- When analyzing long CI logs or test output, save the output to a file and delegate analysis to a subagent with the file content as context
- For test utilities:
  - `test_metric()` - short, focused examples (e.g., doc examples, simple assertions)
    ```rust
    // metrique-writer/src/test_util.rs
    let entry = test_metric(MyMetrics { field: value });
    assert_eq!(entry.metrics["Field"], value);
    ```
  - `test_entry_sink()` - longer-form tests with multiple entries or queue behavior
    ```rust
    // metrique-writer/src/test_util.rs
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let mut metrics = MyMetrics::default().append_on_drop(sink);
    // ... mutate metrics ...
    drop(metrics);
    let entries = inspector.take();
    assert!(entries.iter().any(|e| e.metrics["Field"] == expected));
    ```
  
- Both `metrique` and `metrique-aggregation` have their own set of UI tests in metrique/tests/ui and metrique-aggregation/tests/ui. These both only run on Rust 1.89 (or whatever the current pinned Rust version is for ui tests). The current version is defined in build.yml

## Finishing Up
When instructed to "finish up", follow this process:
1. Run `cargo +nightly fmt --all` to format all code
2. Run `cargo clippy --workspace --all-features -- -D warnings` and fix all errors
3. Fix any small issues found. For big issues, confirm with the user first
4. Amend changes into the most recent commit: `git add -A && git commit --amend --no-edit`
5. Push: `git push --force-with-lease`
6. Monitor CI on GitHub to ensure it passes. If it fails:
   - Analyze why the issue was missed
   - Update the pre-CI checks to catch similar issues
   - Fix the problem and repeat from step 1

Note: For a full local CI check matching what runs on GitHub, use `scripts/ci-local.sh`

Helpful commands for monitoring CI:
- Check PR status: `gh pr checks <branch-name>`
- Watch until completion: `gh pr checks <branch-name> --watch` (refreshes every 10s, exits when done)
- View with details: `gh pr view <branch-name> --json statusCheckRollup --jq '.statusCheckRollup[] | "\(.name): \(.status) - \(.conclusion)"'`

# Metrique Trait System

## Overview

Metrique uses a trait-based system to transform user-defined metric structs into entries that can be emitted. The key insight is that metrics go through a "closing" process where mutable accumulation types (like `Timer`, `Histogram`) are converted into immutable observation types.

## Core Traits

### `CloseValue`
Defined in `metrique-core/src/lib.rs`:
```rust
pub trait CloseValue {
    type Closed;
    fn close(self) -> Self::Closed;
}
```
