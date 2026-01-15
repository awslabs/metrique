use darling::FromField;
use proc_macro2::{Ident, TokenStream as Ts2};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Attribute, Data, DeriveInput, Error, Fields, Result, Type};

#[derive(Debug)]
struct AggregateField {
    name: Ident,
    ty: Type,
    strategy: Option<Type>,
    is_key: bool,
    is_ignored: bool,
    use_clone: bool,
    metrics_attrs: Vec<Attribute>,
}

#[derive(Debug)]
struct ParsedAggregate {
    fields: Vec<AggregateField>,
}

fn parse_aggregate_fields(input: &DeriveInput) -> Result<ParsedAggregate> {
    let data_struct = match &input.data {
        Data::Struct(s) => s,
        _ => return Err(Error::new(input.span(), "aggregate only supports structs")),
    };

    let fields = match &data_struct.fields {
        Fields::Named(f) => &f.named,
        _ => {
            return Err(Error::new(
                input.span(),
                "aggregate only supports named fields",
            ));
        }
    };

    let mut parsed_fields = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    for field in fields {
        let name = field
            .ident
            .clone()
            .expect("unreachable: aggregate only supports named fields, checked above.");

        // Check for duplicate field names
        if !seen_names.insert(name.to_string()) {
            return Err(Error::new(
                name.span(),
                format!("duplicate field name '{}'", name),
            ));
        }

        let mut strategy = None;
        let mut is_key = false;
        let mut is_ignored = false;
        let mut use_clone = false;

        for attr in &field.attrs {
            if attr.path().is_ident("aggregate") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("strategy") {
                        if strategy.is_some() {
                            return Err(meta.error("duplicate 'strategy' attribute"));
                        }
                        let value = meta.value()?;
                        strategy = Some(value.parse()?);
                        Ok(())
                    } else if meta.path.is_ident("key") {
                        if is_key {
                            return Err(meta.error("duplicate 'key' attribute"));
                        }
                        is_key = true;
                        Ok(())
                    } else if meta.path.is_ident("ignore") {
                        is_ignored = true;
                        Ok(())
                    } else if meta.path.is_ident("clone") {
                        if use_clone {
                            return Err(meta.error("duplicate 'clone' attribute"));
                        }
                        use_clone = true;
                        Ok(())
                    } else {
                        let path_str = meta.path.get_ident()
                            .map(|i| i.to_string())
                            .unwrap_or_else(|| meta.path.to_token_stream().to_string());
                        Err(meta.error(format!(
                            "unknown aggregate attribute '{}'. Valid attributes are: strategy, key, clone",
                            path_str
                        )))
                    }
                })?;
            }
        }

        // Check for conflicting attributes
        if is_key && strategy.is_some() {
            return Err(Error::new(
                name.span(),
                format!(
                    "field '{}' cannot have both 'key' and 'strategy' attributes",
                    name
                ),
            ));
        }

        if !is_key && !is_ignored && strategy.is_none() {
            return Err(Error::new(
                name.span(),
                format!(
                    "field '{}' requires #[aggregate(strategy = ...)] attribute or if it is a key, use #[aggregate(key)], or to skip aggregation, use #[aggregate(ignore)]",
                    name
                ),
            ));
        }

        let metrics_attrs = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("metrics"))
            .cloned()
            .collect();

        parsed_fields.push(AggregateField {
            name,
            ty: field.ty.clone(),
            strategy,
            is_key,
            is_ignored,
            use_clone,
            metrics_attrs,
        });
    }

    Ok(ParsedAggregate {
        fields: parsed_fields,
    })
}

pub(crate) fn generate_aggregated_struct(input: &DeriveInput, entry_mode: bool) -> Result<Ts2> {
    let parsed = parse_aggregate_fields(input)?;
    let original_name = &input.ident;
    let aggregated_name = format_ident!("Aggregated{}", original_name);

    let aggregated_fields = parsed.fields.iter().filter(|f| !f.is_key && !f.is_ignored).map(|f| {
        let name = &f.name;
        let metrics_attrs = &f.metrics_attrs;
        let strategy = f.strategy.as_ref().unwrap();
        let source_ty = &f.ty;
        let value_ty = if entry_mode {
            quote! { <#source_ty as metrique::CloseValue>::Closed }
        } else {
            quote! { #source_ty }
        };
        quote! {
            #(#metrics_attrs)*
            #name: <#strategy as ::metrique_aggregation::__macro_plumbing::AggregateValue<#value_ty>>::Aggregated
        }
    }).collect::<Vec<_>>();

    let metrics_attr = input
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("metrics"));

    let derive_default = quote! { #[derive(Default)] };

    // In direct mode, always add #[metrics] if not present
    let metrics_attr = if !entry_mode && metrics_attr.is_none() {
        quote! { #[metrics] }
    } else {
        quote! { #metrics_attr }
    };

    Ok(quote! {
        #metrics_attr
        #derive_default
        #[allow(clippy::type_complexity)]
        // aggregated needs to be pub because it is used in a trait
        pub struct #aggregated_name {
            #(#aggregated_fields),*
        }
    })
}

pub(crate) fn generate_aggregate_strategy_impl(
    input: &DeriveInput,
    entry_mode: bool,
) -> Result<Ts2> {
    let parsed = parse_aggregate_fields(input)?;
    let original_name = &input.ident;
    let aggregated_name = format_ident!("Aggregated{}", original_name);
    let key_name = format_ident!("{}Key", original_name);
    let key_extractor_name = format_ident!("{}KeyExtractor", original_name);
    let vis = &input.vis;

    let key_fields: Vec<_> = parsed.fields.iter().filter(|f| f.is_key).collect();

    // Determine the source type for AggregateStrategy
    let source_ty = if entry_mode {
        quote! { <#original_name as metrique::CloseValue>::Closed }
    } else {
        quote! { #original_name }
    };

    // Generate Merge impl
    let merge_calls = parsed.fields.iter().filter(|f| !f.is_key && !f.is_ignored).map(|f| {
        let name = &f.name;
        let strategy = f.strategy.as_ref().unwrap();
        let field_ty = &f.ty;

        let value_ty = if entry_mode {
            quote! { <#field_ty as metrique::CloseValue>::Closed }
        } else {
            quote! { #field_ty }
        };

        // Check if field has a unit attribute by parsing metrics attributes
        // Only dereference in entry mode, where the field is wrapped in WithUnit
        let has_unit = entry_mode && crate::RawMetricsFieldAttrs::from_field(&syn::Field {
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

        // In entry mode with unit, need to dereference WithUnit wrapper
        let entry_value = if has_unit {
            quote! { *input.#name }
        } else {
            quote! { input.#name }
        };

        let field_span = name.span();

        let expect_deprecated = if entry_mode {
            quote! { #[expect(deprecated)] }
        } else {
            quote! {}
        };

        quote_spanned! { field_span=>
            #expect_deprecated
            <#strategy as ::metrique_aggregation::__macro_plumbing::AggregateValue<#value_ty>>::insert(&mut accum.#name, #entry_value);
        }
    }).collect::<Vec<_>>();

    // Generate Merge impl
    let merge_impl = quote! {
        impl ::metrique_aggregation::__macro_plumbing::Merge for #source_ty {
            type Merged = #aggregated_name;
            type MergeConfig = ();

            fn new_merged(_conf: &Self::MergeConfig) -> Self::Merged {
                Self::Merged::default()
            }

            fn merge(accum: &mut Self::Merged, input: Self) {
                #(#merge_calls)*
            }
        }
    };

    // Generate Key struct and impl if there are key fields
    let (key_struct, key_impl, strategy_key_type) = if key_fields.is_empty() {
        (
            quote! {},
            quote! {},
            quote! { ::metrique_aggregation::__macro_plumbing::NoKey },
        )
    } else {
        let key_field_defs = key_fields.iter().map(|f| {
            let name = &f.name;
            let ty = &f.ty;
            let metrics_attrs = &f.metrics_attrs;
            quote! {
                #(#metrics_attrs)*
                #name: ::std::borrow::Cow<'a, #ty>
            }
        });

        let from_source_fields = key_fields.iter().map(|f| {
            let name = &f.name;
            quote! {
                #name: ::std::borrow::Cow::Borrowed(&source.#name)
            }
        });

        let static_key_fields = key_fields.iter().map(|f| {
            let name = &f.name;
            quote! {
                #name: ::std::borrow::Cow::Owned(key.#name.clone().into_owned())
            }
        });

        let key_struct = quote! {
            #[derive(Clone, Hash, PartialEq, Eq)]
            #[metrics]
            // key struct needs to be pub because it is used in a trait
            pub struct #key_name<'a> {
                #(#key_field_defs),*
            }
        };

        let key_impl = quote! {
            #vis struct #key_extractor_name;

            impl ::metrique_aggregation::__macro_plumbing::Key<#source_ty> for #key_extractor_name {
                type Key<'a> = #key_name<'a>;

                fn from_source(source: &#source_ty) -> Self::Key<'_> {
                    #[allow(deprecated)]
                    #key_name {
                        #(#from_source_fields),*
                    }
                }

                fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
                    #key_name {
                        #(#static_key_fields),*
                    }
                }

                fn static_key_matches<'a>(owned: &Self::Key<'static>, borrowed: &Self::Key<'a>) -> bool {
                    owned == borrowed
                }
            }
        };

        (key_struct, key_impl, quote! { #key_extractor_name })
    };

    // Generate AggregateStrategy impl
    let strategy_impl = quote! {
        impl ::metrique_aggregation::__macro_plumbing::AggregateStrategy for #original_name {
            type Source = #source_ty;
            type Key = #strategy_key_type;
        }
    };

    Ok(quote! {
        #merge_impl
        #key_struct
        #key_impl
        #strategy_impl
    })
}

pub(crate) fn generate_merge_ref_impl(
    input: &DeriveInput,
    entry_mode: bool,
    enable_merge_ref: bool,
) -> Result<Option<Ts2>> {
    let parsed = parse_aggregate_fields(input)?;

    // Only generate if #[aggregate(ref)] is present
    if !enable_merge_ref {
        return Ok(None);
    }

    let original_name = &input.ident;

    // Determine the source type
    let source_ty = if entry_mode {
        quote! { <#original_name as metrique::CloseValue>::Closed }
    } else {
        quote! { #original_name }
    };

    // Generate merge_ref calls for non-key fields
    let merge_ref_calls = parsed.fields.iter().filter(|f| !f.is_key && !f.is_ignored).map(|f| {
        let name = &f.name;
        let strategy = f.strategy.as_ref().unwrap();
        let field_ty = &f.ty;

        let value_ty = if entry_mode {
            quote! { <#field_ty as metrique::CloseValue>::Closed }
        } else {
            quote! { #field_ty }
        };

        let field_span = name.span();

        let expect_deprecated = if entry_mode {
            quote! { #[expect(deprecated)] }
        } else {
            quote! {}
        };

        if f.use_clone {
            // Use clone for this field
            quote_spanned! { field_span=>
                #expect_deprecated
                <#strategy as ::metrique_aggregation::__macro_plumbing::AggregateValue<#value_ty>>::insert(&mut accum.#name, input.#name.clone());
            }
        } else {
            // Use CopyWrapper for Copy types
            quote_spanned! { field_span=>
                #expect_deprecated
                <::metrique_aggregation::__macro_plumbing::CopyWrapper<#strategy> as ::metrique_aggregation::__macro_plumbing::AggregateValue<&#value_ty>>::insert(&mut accum.#name, &input.#name);
            }
        }
    }).collect::<Vec<_>>();

    let merge_ref_impl = quote! {
        impl ::metrique_aggregation::__macro_plumbing::MergeRef for #source_ty {
            fn merge_ref(accum: &mut Self::Merged, input: &Self) {
                #(#merge_ref_calls)*
            }
        }
    };

    Ok(Some(merge_ref_impl))
}

pub(crate) fn generate_merge_on_drop_methods(input: &DeriveInput, entry_mode: bool) -> Result<Ts2> {
    let original_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    if entry_mode {
        // Default mode: generate close_and_merge method
        Ok(quote! {
            impl #impl_generics #original_name #ty_generics #where_clause {
                /// Create a guard that will close and merge this value on drop
                pub fn close_and_merge<Sink: ::metrique_aggregation::traits::RootSink<<Self as metrique::CloseValue>::Closed>>(
                    self,
                    sink: Sink,
                ) -> ::metrique_aggregation::sink::CloseAndMergeOnDrop<Self, Sink> {
                    ::metrique_aggregation::sink::CloseAndMergeOnDrop::new(self, sink)
                }
            }
        })
    } else {
        // Raw mode: generate merge method
        Ok(quote! {
            impl #impl_generics #original_name #ty_generics #where_clause {
                /// Create a guard that will merge this value on drop
                pub fn merge<Sink: ::metrique_aggregation::traits::RootSink<Self>>(
                    self,
                    sink: Sink,
                ) -> ::metrique_aggregation::sink::MergeOnDrop<Self, Sink> {
                    ::metrique_aggregation::sink::MergeOnDrop::new(self, sink)
                }
            }
        })
    }
}

pub(crate) fn clean_aggregate_adt(input: &DeriveInput) -> Ts2 {
    let adt_name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;

    let filtered_attrs = clean_aggregate_attrs(&input.attrs);
    match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => {
                let fields = fields_named.named.iter().map(|f| {
                    let name = &f.ident;
                    let ty = &f.ty;
                    let vis = &f.vis;
                    let attrs = clean_aggregate_attrs(&f.attrs);
                    quote! {
                        #(#attrs)*
                        #vis #name: #ty
                    }
                });
                quote! {
                    #(#filtered_attrs)*
                    #vis struct #adt_name #generics {
                        #(#fields),*
                    }
                }
            }
            _ => input.to_token_stream(),
        },
        _ => input.to_token_stream(),
    }
}

fn clean_aggregate_attrs(attr: &[Attribute]) -> Vec<Attribute> {
    attr.iter()
        .filter(|attr| !attr.path().is_ident("aggregate"))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse2;

    fn aggregate_impl(input: Ts2, entry_mode: bool) -> Ts2 {
        let input = syn::parse2(input).unwrap();
        let mut output = Ts2::new();

        if let Ok(aggregated_struct) = generate_aggregated_struct(&input, entry_mode) {
            output.extend(aggregated_struct);
        }

        if let Ok(aggregate_impl) = generate_aggregate_strategy_impl(&input, entry_mode) {
            output.extend(aggregate_impl);
        }

        output.extend(clean_aggregate_adt(&input));
        output
    }

    fn aggregate_impl_string(input: Ts2) -> String {
        let output = aggregate_impl(input, false);
        match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => output.to_string(),
        }
    }

    #[test]
    fn test_aggregate_strips_attributes() {
        let input = quote! {
            #[metrics]
            pub struct ApiCall {
                #[aggregate(strategy = Histogram<Duration>)]
                #[metrics(unit = Millisecond)]
                latency: Duration,
                #[aggregate(strategy = Counter)]
                #[metrics(unit = Byte)]
                response_size: usize,
            }
        };

        let parsed_file = aggregate_impl_string(input);
        insta::assert_snapshot!("aggregate_strips_attributes", parsed_file);
    }

    #[test]
    fn test_aggregate_generates_struct() {
        let input = quote! {
            #[metrics]
            pub struct ApiCall {
                #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
                #[metrics(unit = Millisecond, name = "latency_2")]
                latency: Duration,
                #[aggregate(strategy = Counter)]
                #[metrics(unit = Byte)]
                response_size: usize,
                #[aggregate(strategy = MergeOptions<KeepLast>)]
                response_value: Option<String>,
            }
        };

        let parsed_file = aggregate_impl_string(input);
        insta::assert_snapshot!("aggregate_generates_struct", parsed_file);
    }

    #[test]
    fn test_aggregate_with_key() {
        let input = quote! {
            #[metrics]
            struct ApiCallWithOperation {
                #[aggregate(key)]
                endpoint: String,
                #[aggregate(strategy = Histogram<Duration>)]
                #[metrics(unit = Millisecond)]
                latency: Duration,
            }
        };

        let parsed_file = aggregate_impl_string(input);
        insta::assert_snapshot!("aggregate_with_key", parsed_file);
    }

    #[test]
    fn test_aggregate_entry_mode() {
        let input = quote! {
            #[metrics]
            struct ApiCall {
                #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
                #[metrics(unit = Millisecond, name = "latency_2")]
                latency: Timer,
            }
        };

        let output = aggregate_impl(input, true);
        let parsed_file = match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => output.to_string(),
        };
        insta::assert_snapshot!("aggregate_entry_mode", parsed_file);
    }

    #[test]
    fn test_aggregate_direct_mode() {
        let input = quote! {
            struct RawData {
                #[aggregate(strategy = Histogram<Duration>)]
                latency: Duration,
                #[aggregate(strategy = Sum)]
                count: u64,
            }
        };

        let output = aggregate_impl(input, false);
        let parsed_file = match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => output.to_string(),
        };
        insta::assert_snapshot!("aggregate_direct_mode", parsed_file);
    }

    #[test]
    fn test_aggregate_with_ignore() {
        let input = quote! {
            #[metrics]
            struct ApiCall {
                #[aggregate(strategy = Histogram<Duration>)]
                #[metrics(unit = Millisecond)]
                latency: Duration,
                #[aggregate(ignore)]
                request_id: String,
            }
        };

        let parsed_file = aggregate_impl_string(input);
        insta::assert_snapshot!("aggregate_with_ignore", parsed_file);
    }

    #[test]
    fn test_unknown_attribute() {
        use assert2::check;

        let input = quote! {
            struct ApiCall {
                #[aggregate(unknown_attr)]
                latency: Duration,
            }
        };

        let input = syn::parse2(input).unwrap();
        let result = parse_aggregate_fields(&input);
        check!(result.is_err());
        let err = result.unwrap_err().to_string();
        check!(err.contains("unknown aggregate attribute 'unknown_attr'"));
        check!(err.contains("Valid attributes are: strategy, key, clone"));
    }

    #[test]
    fn test_duplicate_strategy() {
        use assert2::check;

        let input = quote! {
            struct ApiCall {
                #[aggregate(strategy = Counter, strategy = Sum)]
                count: u64,
            }
        };

        let input = syn::parse2(input).unwrap();
        let result = parse_aggregate_fields(&input);
        check!(result.is_err());
        check!(
            result
                .unwrap_err()
                .to_string()
                .contains("duplicate 'strategy' attribute")
        );
    }

    #[test]
    fn test_duplicate_key() {
        use assert2::check;

        let input = quote! {
            struct ApiCall {
                #[aggregate(key, key)]
                endpoint: String,
            }
        };

        let input = syn::parse2(input).unwrap();
        let result = parse_aggregate_fields(&input);
        check!(result.is_err());
        check!(
            result
                .unwrap_err()
                .to_string()
                .contains("duplicate 'key' attribute")
        );
    }

    #[test]
    fn test_conflicting_key_and_strategy() {
        use assert2::check;

        let input = quote! {
            struct ApiCall {
                #[aggregate(key, strategy = Counter)]
                endpoint: String,
            }
        };

        let input = syn::parse2(input).unwrap();
        let result = parse_aggregate_fields(&input);
        check!(result.is_err());
        check!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot have both 'key' and 'strategy' attributes")
        );
    }

    #[test]
    fn test_missing_strategy_or_key() {
        use assert2::check;

        let input = quote! {
            struct ApiCall {
                #[aggregate()]
                latency: Duration,
            }
        };

        let input = syn::parse2(input).unwrap();
        let result = parse_aggregate_fields(&input);
        check!(result.is_err());
        check!(
            result
                .unwrap_err()
                .to_string()
                .contains("requires #[aggregate(strategy = ...)]")
        );
    }
}
