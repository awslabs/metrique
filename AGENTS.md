# Metrique Workspace Guidelines

## Testing
- Use `cargo +1.89 nextest run` to run all tests in this workspace
- If there are mismatches in trybuild or insta snapshots, share the diff for user approval before accepting them
- Before commiting run `cargo fmt` and `cargo clippy`. YOU MUST FIX CLIPPY ERRORS.
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
