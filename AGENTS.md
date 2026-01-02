# Metrique Trait System

# Ways of working (IMPORTANT)

- When running all tests, use `cargo nextest run`
- Before commiting run `cargo fmt` and `cargo clippy`. YOU MUST FIX CLIPPY ERRORS.

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
