Our next task is creating a parallel path between Entry/CloseEntry and aggregation.

Right now, the `Source` type of `AggregateStrategy` is the "user type":

```rust
#[metrics]
struct MyStruct {
    // ...
}
```

Right now, during the merge, step, the inner values are closed. What we want _instead_ is that the `Source` type is the `Entry` type (aka, the resolve of `MyStruct::Closed`):

```
struct MyStructStrategy;
impl AggregateStrategy for MyStructStrategy {
  type Source = MyStruct::Closed;
  // ...
}

In the `#[aggregate]` macro, unless `#[aggregate(no_close)]` is used (this is currently "raw"), we should be aggregating on the closed type.

I've set up the `CloseAggregateEntry` trait and added `Aggregate2` demonstrating how it will be used. In the future we'll have `Aggregate` and `AggregateRaw` -- `Aggregate` accepts something with `Close.

Actions:
- Rename `Aggregate` to `AggregateRaw`
- Rename `Aggreagte2` to `Aggregate`

### Tricky Issues
#### Units
1. Dealing with `WithUnit` — right now, when you have a struct like this:
```rust
[#metrics]
struct MyThing {
  #[metrics(unit = Millisecon)]
  a: Duration
}
```

The `Entry` type actually has `WithUnit<Duration, Millisecond>` which is somewhat annoying. When you try to merge those fields, it doesn't work as expected.

Since `WithUnit` impls `Deref`, the fix is to deref, _ONLY_ when unit is present.
```
        // Check if field has a unit attribute by parsing metrics attributes
        // Only dereference in entry mode, where the field is wrapped in WithUnit
        let has_unit = entry_mode && RawMetricsFieldAttrs::from_field(&syn::Field {
            attrs: f.metrics_attrs.clone(),
            vis: syn::Visibility::Inherited,
            mutability: syn::FieldMutability::None,
            ident: Some(f.name.clone()),
            colon_token: None,
            ty: f.ty.clone(),
        })
        .ok()
        .and_then(|attrs| attrs.unit)
        .is_some();

        let entry_value = if has_unit {
            quote! { *entry.#name }
        } else {
            quote! { entry.#name }
        };
```

If you look at the state of code from this commit, you can see the e2e structure working (although, with the old trait structure.)

### Preserving Units and Names
You need to ensure that in the aggregated struct you generate, you preserve the `#[metrics]` attributes attached to individual fields. These are critical to ensuring that the resulting aggregation _has_ units attached properly.

#### Deprecated fields
The inner entry fields are marked as deprecated. You need to add an `#[expect(deprecated)]` before the call to `merge`:
Example from old code:
```rust
#[allow(deprecated)]
     <#strategy as metrique_aggregation::aggregate::AggregateValue<#value_ty>>::add_value(
         &mut accum.#name,
         entry.#name,
         #entry_value,
     );
```

---

Here are tools available to you:
1. The unit tests in ../metrique-macro are useful for seeing what the macro is generating -- but beware! Since they don't actually compile the code, its easy to go deep down wrong paths.

2. Once you are happy with the individual macro, make a _NEW_ integration test in `tests/aggregation_new.rs`. Build up slowly to incrementally test the features that exist in the `aggregation` test today.

2. Other integration tests in the metrique-aggregation crate

3. Me!

Be sure to ask any clarifying questions. If your understanding changes, update this doc.
