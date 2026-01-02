use crate::RawMetricsFieldAttrs;
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
    metrics_attrs: Vec<Attribute>,
}

#[derive(Debug)]
struct ParsedAggregate {
    fields: Vec<AggregateField>,
    has_key: bool,
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
    let mut has_key = false;

    for field in fields {
        let name = field
            .ident
            .clone()
            .ok_or_else(|| Error::new(field.span(), "aggregate only supports named fields"))?;

        let mut strategy = None;
        let mut is_key = false;

        for attr in &field.attrs {
            if attr.path().is_ident("aggregate") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("strategy") {
                        let value = meta.value()?;
                        strategy = Some(value.parse()?);
                        Ok(())
                    } else if meta.path.is_ident("key") {
                        is_key = true;
                        Ok(())
                    } else {
                        Err(meta.error("unknown aggregate attribute"))
                    }
                })?;
            }
        }

        if !is_key && strategy.is_none() {
            return Err(Error::new(
                name.span(),
                format!(
                    "field '{}' requires #[aggregate(strategy = ...)] attribute",
                    name
                ),
            ));
        }

        if is_key {
            has_key = true;
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
            metrics_attrs,
        });
    }

    Ok(ParsedAggregate {
        fields: parsed_fields,
        has_key,
    })
}

pub(crate) fn generate_aggregated_struct(input: &DeriveInput, entry_mode: bool) -> Result<Ts2> {
    let parsed = parse_aggregate_fields(input)?;
    let original_name = &input.ident;
    let aggregated_name = format_ident!("Aggregated{}", original_name);
    let vis = &input.vis;

    let aggregated_fields = parsed.fields.iter().map(|f| {
        let name = &f.name;
        let metrics_attrs = &f.metrics_attrs;

        if f.is_key {
            let ty = &f.ty;
            quote! {
                #(#metrics_attrs)*
                #name: #ty
            }
        } else {
            let strategy = f.strategy.as_ref().unwrap();
            let source_ty = &f.ty;
            let value_ty = if entry_mode {
                quote! { <#source_ty as metrique::CloseValue>::Closed }
            } else {
                quote! { #source_ty }
            };
            quote! {
                #(#metrics_attrs)*
                #name: <#strategy as metrique_aggregation::__macro_plumbing::AggregateValue<#value_ty>>::Aggregated
            }
        }
    }).collect::<Vec<_>>();

    let metrics_attr = input
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("metrics"));

    let derive_default = if !parsed.has_key {
        quote! { #[derive(Default)] }
    } else {
        quote! {}
    };

    Ok(quote! {
        #metrics_attr
        #derive_default
        #vis struct #aggregated_name {
            #(#aggregated_fields),*
        }
    })
}

pub(crate) fn generate_aggregate_entry_impl(
    input: &DeriveInput,
    entry_mode: bool,
    owned_mode: bool,
) -> Result<Ts2> {
    let parsed = parse_aggregate_fields(input)?;
    let original_name = &input.ident;
    let aggregated_name = format_ident!("Aggregated{}", original_name);

    let key_fields: Vec<_> = parsed.fields.iter().filter(|f| f.is_key).collect();

    let (key_type, key_expr, static_key_impl, new_aggregated_body) = if key_fields.is_empty() {
        (
            quote! { () },
            quote! { () },
            quote! { () },
            quote! { Self::Aggregated::default() },
        )
    } else {
        let key_borrowed_refs = key_fields.iter().map(|f| {
            let name = &f.name;
            quote! { ::std::borrow::Cow::Borrowed(&source.#name) }
        });
        let key_type_refs = key_fields.iter().map(|f| {
            let ty = &f.ty;
            quote! { ::std::borrow::Cow<'a, #ty> }
        });
        let key_type = if key_fields.len() == 1 {
            quote! { #(#key_type_refs)* }
        } else {
            quote! { (#(#key_type_refs),*) }
        };
        let key_expr = if key_fields.len() == 1 {
            quote! { #(#key_borrowed_refs)* }
        } else {
            quote! { (#(#key_borrowed_refs),*) }
        };

        let static_key_conversion = if key_fields.len() == 1 {
            quote! { ::std::borrow::Cow::Owned(key.into_owned()) }
        } else {
            let conversions = (0..key_fields.len()).map(|i| {
                let idx = syn::Index::from(i);
                quote! { ::std::borrow::Cow::Owned(key.#idx.into_owned()) }
            });
            quote! { (#(#conversions),*) }
        };

        let field_inits = parsed.fields.iter().map(|f| {
            let name = &f.name;
            if f.is_key {
                if key_fields.len() == 1 {
                    quote! { #name: key.clone().into_owned() }
                } else {
                    let idx = syn::Index::from(
                        key_fields.iter().position(|kf| kf.name == f.name).unwrap(),
                    );
                    quote! { #name: key.#idx.clone().into_owned() }
                }
            } else {
                quote! { #name: Default::default() }
            }
        });

        (
            key_type,
            key_expr,
            static_key_conversion,
            quote! {
                #aggregated_name {
                    #(#field_inits),*
                }
            },
        )
    };

    let merge_calls = parsed.fields.iter().filter(|f| !f.is_key).map(|f| {
        let name = &f.name;
        let source_ty = &f.ty;
        let strategy = f.strategy.as_ref().unwrap();

        let value_ty = if entry_mode {
            quote! { <#source_ty as metrique::CloseValue>::Closed }
        } else {
            quote! { #source_ty }
        };

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

        let field_span = name.span();

        quote_spanned! { field_span=>
            #[allow(deprecated)]
            <#strategy as metrique_aggregation::__macro_plumbing::AggregateValue<#value_ty>>::add_value(
                &mut accum.#name,
                #entry_value,
            );
        }
    }).collect::<Vec<_>>();

    let merge_calls_ref = parsed.fields.iter().filter(|f| !f.is_key).map(|f| {
        let name = &f.name;
        let source_ty = &f.ty;
        let strategy = f.strategy.as_ref().unwrap();

        let value_ty = if entry_mode {
            quote! { <#source_ty as metrique::CloseValue>::Closed }
        } else {
            quote! { #source_ty }
        };

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
            quote! { &*entry.#name }
        } else {
            quote! { &entry.#name }
        };

        let field_span = name.span();

        quote_spanned! { field_span=>
            #[allow(deprecated)]
            <metrique_aggregation::__macro_plumbing::IfYouSeeThisUseAggregateOwned::<#strategy> as metrique_aggregation::__macro_plumbing::AggregateValue<&#value_ty>>::add_value(
                &mut accum.#name,
                #entry_value,
            );
        }
    }).collect::<Vec<_>>();

    let source_type = if entry_mode {
        quote! { <Self as metrique::CloseValue>::Closed }
    } else {
        quote! { Self }
    };

    if owned_mode {
        Ok(quote! {
            impl metrique_aggregation::__macro_plumbing::MergeOnDropExt for #original_name {}

            impl metrique_aggregation::__macro_plumbing::AggregateEntry for #original_name {
                type Source = #source_type;
                type Aggregated = #aggregated_name;
                type Key<'a> = #key_type;

                fn static_key<'a>(key: Self::Key<'a>) -> Self::Key<'static> {
                    #static_key_impl
                }

                fn merge_entry(accum: &mut Self::Aggregated, entry: Self::Source) {
                    #(#merge_calls)*
                }

                fn new_aggregated<'a>(key: &Self::Key<'a>) -> Self::Aggregated {
                    #new_aggregated_body
                }

                fn key(source: &Self::Source) -> Self::Key<'_> {
                    #[allow(deprecated)]
                    #key_expr
                }
            }
        })
    } else {
        Ok(quote! {
            impl metrique_aggregation::__macro_plumbing::MergeOnDropExt for #original_name {}

            impl metrique_aggregation::__macro_plumbing::AggregateEntry for #original_name {
                type Source = #source_type;
                type Aggregated = #aggregated_name;
                type Key<'a> = #key_type;

                fn static_key<'a>(key: Self::Key<'a>) -> Self::Key<'static> {
                    #static_key_impl
                }

                fn merge_entry(accum: &mut Self::Aggregated, entry: Self::Source) {
                    <Self as metrique_aggregation::__macro_plumbing::AggregateEntryRef>::merge_entry_ref(accum, &entry);
                }

                fn new_aggregated<'a>(key: &Self::Key<'a>) -> Self::Aggregated {
                    #new_aggregated_body
                }

                fn key(source: &Self::Source) -> Self::Key<'_> {
                    #[allow(deprecated)]
                    #key_expr
                }
            }

            impl metrique_aggregation::__macro_plumbing::AggregateEntryRef for #original_name {
                fn merge_entry_ref(accum: &mut Self::Aggregated, entry: &Self::Source) {
                    #(#merge_calls_ref)*
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

        if let Ok(aggregate_impl) = generate_aggregate_entry_impl(&input, entry_mode, true) {
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
                #[aggregate(strategy = MergeOptions<LastValueWins>)]
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
}
