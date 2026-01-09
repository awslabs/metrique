# MergeRef Implementation Plan

## Overview
Add `MergeRef` trait implementation generation to the `#[aggregate]` macro, enabling reference-based aggregation for efficient multi-sink patterns like `SplitSink`.

## 1. Locate and Document `IfYouSeeThisUseAggregateOwned`

**Location**: Search codebase for existing implementation

**Purpose**: Wrapper strategy that enables `Copy` types to be aggregated by reference by dereferencing and calling the underlying strategy.

**Expected signature**:
```rust
pub struct IfYouSeeThisUseAggregateOwned<S>(PhantomData<S>);

impl<T: Copy, S: AggregateValue<T>> AggregateValue<&T> for IfYouSeeThisUseAggregateOwned<S> {
    type Aggregated = S::Aggregated;
    
    fn add_value(accum: &mut Self::Aggregated, value: &T) {
        S::add_value(accum, *value)
    }
}
```

## 2. Update `#[aggregate]` Macro

**Location**: `metrique-macro` crate (proc macro implementation)

### 2.1 Add Attribute Parsing

**New attributes**:
- `#[aggregate(owned)]` - on struct: disables `MergeRef` generation entirely
- `#[aggregate(clone)]` - on field: uses `.clone()` instead of `Copy` wrapper for that field

### 2.2 Generate `MergeRef` Implementation

**Default behavior**: Always generate `MergeRef` unless `#[aggregate(owned)]` is present

**Generated code pattern**:
```rust
impl MergeRef for ApiCall {
    fn merge_ref(accum: &mut Self::Merged, input: &Self) {
        // For Copy fields (default):
        <IfYouSeeThisUseAggregateOwned<Histogram<Duration>> as AggregateValue<&Duration>>::add_value(
            &mut accum.latency, 
            &input.latency
        );
        
        // For fields marked with #[aggregate(clone)]:
        <SomeStrategy as AggregateValue<String>>::add_value(
            &mut accum.field_name,
            input.field_name.clone()
        );
    }
}
```

### 2.3 Compilation Error Handling

**For non-Copy fields without `#[aggregate(clone)]`**:
- The generated code will fail to compile with a clear error
- Error will indicate that `IfYouSeeThisUseAggregateOwned` requires `Copy`
- User must either:
  - Add `#[aggregate(clone)]` to the field
  - Add `#[aggregate(owned)]` to the struct
  - Make the field `Copy`

## 3. Macro Implementation Details

### Field Processing Logic

```rust
for field in fields {
    if has_aggregate_clone_attr(field) {
        // Generate: strategy::add_value(&mut accum.field, input.field.clone())
        generate_clone_based_merge(field)
    } else {
        // Generate: IfYouSeeThisUseAggregateOwned<strategy>::add_value(&mut accum.field, &input.field)
        generate_copy_based_merge(field)
    }
}
```

### Opt-out Check

```rust
if has_aggregate_owned_attr(struct_attrs) {
    // Don't generate MergeRef impl
    return;
}
```

## 4. Enable `SplitSink` Test

**Location**: `tests/split_sink.rs`

**Changes**:
1. Remove `#![cfg(feature = "never_enabled")]`
2. Test should compile and pass once macro generates `MergeRef`

## 5. Add Test Coverage

### 5.1 Basic MergeRef Generation Test
**Location**: New test in `tests/aggregation.rs` or separate file

```rust
#[aggregate]
#[metrics]
struct AllCopyFields {
    #[aggregate(strategy = Sum)]
    count: u64,
    
    #[aggregate(strategy = Histogram<Duration>)]
    latency: Duration,
}

#[test]
fn test_merge_ref_generated() {
    // Verify MergeRef is implemented
    fn assert_merge_ref<T: MergeRef>() {}
    assert_merge_ref::<AllCopyFieldsEntry>();
}
```

### 5.2 Clone Field Test
```rust
#[aggregate]
#[metrics]
struct WithCloneField {
    #[aggregate(strategy = Sum)]
    count: u64,
    
    #[aggregate(clone)]
    #[aggregate(strategy = LastValueWins)]
    name: String,
}
```

### 5.3 Opt-out Test
```rust
#[aggregate(owned)]
#[metrics]
struct NoMergeRef {
    #[aggregate(strategy = Sum)]
    count: u64,
}

#[test]
fn test_no_merge_ref() {
    // Should NOT implement MergeRef
    // This would fail to compile if MergeRef was generated
}
```

### 5.4 UI Test for Compilation Errors
**Location**: `tests/ui/merge_ref_non_copy.rs`

```rust
#[aggregate]
#[metrics]
struct NonCopyWithoutClone {
    #[aggregate(strategy = LastValueWins)]
    name: String, // Should fail: String is not Copy and no #[aggregate(clone)]
}
```

Expected error should mention `IfYouSeeThisUseAggregateOwned` and `Copy` trait.

## 6. Documentation Updates

### 6.1 README.md

Add section after "Aggregation patterns":

```markdown
### Reference-based aggregation with `MergeRef`

When all fields in an aggregated struct are `Copy`, the `#[aggregate]` macro automatically 
generates a `MergeRef` implementation. This enables efficient multi-sink patterns:

```rust
#[aggregate]
#[metrics]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,
    
    #[aggregate(strategy = Histogram<Duration>)]
    latency: Duration,  // Duration is Copy
}

// Use with SplitSink to aggregate to multiple destinations
let split = SplitSink::new(aggregator_a, aggregator_b);
split.add(api_call.close());  // Both aggregators receive the same data
```

#### Non-Copy Fields

For non-Copy fields, use `#[aggregate(clone)]`:

```rust
#[aggregate]
#[metrics]
struct WithString {
    #[aggregate(strategy = Sum)]
    count: u64,
    
    #[aggregate(clone)]
    #[aggregate(strategy = LastValueWins)]
    name: String,  // Will call .clone() during merge_ref
}
```

#### Opting Out

To disable `MergeRef` generation entirely:

```rust
#[aggregate(owned)]
#[metrics]
struct NoRefAggregation {
    // MergeRef will not be generated
}
```
```

### 6.2 Trait Documentation

Update `MergeRef` trait docs in `src/traits.rs`:

```rust
/// Borrowed version of [`Merge`] for more efficient aggregation.
///
/// When the source type can be borrowed during merging, it becomes possible
/// to aggregate the same input across multiple sinks (or to send it to multiple sinks.)
///
/// The `#[aggregate]` macro automatically generates this implementation when all fields
/// are `Copy`, or when fields are marked with `#[aggregate(clone)]`.
///
/// # Example
///
/// ```rust
/// # use metrique_aggregation::{aggregate, traits::MergeRef};
/// # use metrique::unit_of_work::metrics;
/// # use std::time::Duration;
/// #[aggregate]
/// #[metrics]
/// struct ApiCall {
///     latency: Duration,  // Copy type - MergeRef auto-generated
/// }
/// ```
pub trait MergeRef: Merge {
    /// Merge borrowed input into accumulator
    fn merge_ref(accum: &mut Self::Merged, input: &Self);
}
```

## 7. Implementation Order

1. **Phase 1**: Locate/verify `IfYouSeeThisUseAggregateOwned` exists
2. **Phase 2**: Update macro to parse `#[aggregate(owned)]` and `#[aggregate(clone)]` attributes
3. **Phase 3**: Generate `MergeRef` impl with Copy-based logic (using wrapper)
4. **Phase 4**: Add `#[aggregate(clone)]` field handling
5. **Phase 5**: Add tests (basic, clone, opt-out)
6. **Phase 6**: Enable `SplitSink` test
7. **Phase 7**: Add UI test for error cases
8. **Phase 8**: Update documentation

## 8. Success Criteria

- [ ] `#[aggregate]` generates `MergeRef` by default
- [ ] `#[aggregate(owned)]` prevents `MergeRef` generation
- [ ] `#[aggregate(clone)]` on fields uses `.clone()` in `merge_ref`
- [ ] Non-Copy fields without `#[aggregate(clone)]` produce clear compile errors
- [ ] `SplitSink` test passes
- [ ] All new tests pass
- [ ] Documentation is complete and accurate

## 9. Edge Cases to Consider

1. **Empty structs**: Should still generate `MergeRef` (no fields to merge)
2. **Key-only structs**: Fields marked with `#[aggregate(key)]` are not merged, only non-key fields matter
3. **Mixed Copy/Clone**: Some fields Copy, some Clone - should work fine
4. **Nested aggregation**: If a field's strategy itself requires special handling

## 10. Future Enhancements (Out of Scope)

- Support for custom `MergeRef` implementations on strategies
- Automatic detection of types that implement `Clone` but not `Copy`
- Performance optimization: avoid wrapper overhead for simple cases
