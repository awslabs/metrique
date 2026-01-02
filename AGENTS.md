# Metrique Workspace Guidelines

## Testing
- Use `cargo +1.89 nextest run` to run all tests in this workspace
- If there are mismatches in trybuild or insta snapshots, share the diff for user approval before accepting them
- Before commiting run `cargo fmt` and `cargo clippy`. YOU MUST FIX CLIPPY ERRORS.

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
