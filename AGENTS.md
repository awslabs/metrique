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

This trait converts a mutable accumulation type into an immutable closed type. Examples:
- `Timer` → `Duration`
- `Histogram<T, S>` → `HistogramClosed`
- Primitives like `u64` → `u64` (identity)

### `AttachUnit`
Defined in `metrique/src/lib.rs`:
```rust
pub trait AttachUnit: Sized {
    type Output<U>;
    fn make<U>(self) -> Self::Output<U>;
}
```

This trait wraps a closed value with unit information. The blanket impl:
```rust
impl<V: MetricValue> AttachUnit for V {
    type Output<U> = WithUnit<V, U>;
    fn make<U>(self) -> Self::Output<U> {
        WithUnit::from(self)
    }
}
```

### `WithUnit<V, U>`
Defined in `metrique-writer-core/src/unit.rs`:
```rust
pub struct WithUnit<V, U> {
    value: V,
    _unit_tag: PhantomData<U>,
}
```

This wraps a value with a unit type tag. It implements `Deref` to the inner value and `Value` to write with the correct unit.

## Macro Expansion Flow

When you write:
```rust
#[metrics]
struct TestMetrics {
    #[metrics(unit = Millisecond)]
    latency: Histogram<u32, LinearAggregationStrategy>,
}
```

The macro generates:
```rust
struct TestMetricsEntry {
    latency: <<Histogram<u32, LinearAggregationStrategy> as CloseValue>::Closed 
              as AttachUnit>::Output<Millisecond>
}
```

This expands to:
1. `Histogram<u32, LinearAggregationStrategy>` closes to `HistogramClosed`
2. `HistogramClosed` implements `AttachUnit` (via blanket impl on `MetricValue`)
3. `AttachUnit::Output<Millisecond>` = `WithUnit<HistogramClosed, Millisecond>`

## What Needs to Happen

For `Histogram` to work with `#[metrics(unit = U)]`:

1. **Remove `U` from `Histogram<T, U, S>`** - the unit should come from the macro attribute, not the type
2. **`HistogramClosed` must implement `MetricValue`** - so the blanket `AttachUnit` impl applies
3. **`HistogramClosed` must implement `Value`** - but WITHOUT hardcoding a unit
4. **The unit comes from `WithUnit<HistogramClosed, U>`** - which wraps the closed histogram

## Current Problem

Right now:
- `Histogram<T, U, S>` has `U` as a type parameter
- `HistogramClosed<U>` hardcodes the unit via `U::UNIT`
- This conflicts with the macro's `AttachUnit` system

## Solution

1. Change `Histogram<T, S>` (remove `U`)
2. Change `HistogramClosed` (no unit parameter)
3. Implement `MetricValue` for `HistogramClosed` that writes observations WITHOUT a unit
4. Let `WithUnit<HistogramClosed, U>` handle the unit attachment
