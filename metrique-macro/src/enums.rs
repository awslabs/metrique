// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use darling::{FromField, FromVariant};
use proc_macro2::TokenStream as Ts2;
use quote::quote;
use syn::{Attribute, Generics, Ident, Result, Visibility, spanned::Spanned};

use crate::{MetricMode, TupleData, generate_on_drop_wrapper};
use crate::{
    MetricsField, MetricsFieldKind, RawMetricsFieldAttrs, RootAttributes, SpannedKv, clean_attrs,
    parse_metric_fields, value_impl,
};

/// Indicates how we should parse and validate the variant:
///
/// - ValueString - generate a simple value impl for a unit variant
/// - Entry - full data parsing, tuple/struct variant
/// - SkipAttributeParsing - we are in a cleanup section, no validations
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum VariantMode {
    ValueString,
    Entry,
    SkipAttributeParsing,
}

#[derive(Debug, FromVariant)]
#[darling(attributes(metrics))]
struct RawMetricsVariantAttrs {
    #[darling(default)]
    name: Option<SpannedKv<String>>,
}

impl RawMetricsVariantAttrs {
    fn validate(self, _mode: VariantMode) -> darling::Result<MetricsVariantAttrs> {
        Ok(MetricsVariantAttrs {
            name: self.name.map(|n| n.value),
        })
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct MetricsVariantAttrs {
    pub(crate) name: Option<String>,
}

pub(crate) struct MetricsVariant {
    pub(crate) ident: Ident,
    pub(crate) external_attrs: Vec<Attribute>,
    pub(crate) attrs: MetricsVariantAttrs,
    pub(crate) data: Option<VariantData>,
}
pub(crate) enum VariantData {
    Tuple(Vec<TupleData>),
    Struct(Vec<MetricsField>),
}

impl MetricsVariant {
    pub(crate) fn core_variant(&self) -> Ts2 {
        let MetricsVariant {
            ref external_attrs,
            ref ident,
            ref data,
            ..
        } = *self;

        match data {
            None => quote! { #(#external_attrs)* #ident },
            Some(VariantData::Tuple(tuple_data)) => {
                let types = tuple_data.iter().map(|td| &td.ty);
                quote! { #(#external_attrs)* #ident(#(#types),*) }
            }
            Some(VariantData::Struct(fields)) => {
                let field_defs = fields.iter().map(|f| f.core_field(true));
                quote! { #(#external_attrs)* #ident { #(#field_defs),* } }
            }
        }
    }

    pub(crate) fn entry_variant(&self) -> Ts2 {
        let ident_span = self.ident.span();
        let ident = &self.ident;

        match &self.data {
            None => {
                quote::quote_spanned! { ident_span=>
                    #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`")]
                    #[doc(hidden)]
                    #ident
                }
            }
            Some(VariantData::Tuple(tuple_data)) => {
                let entry_types: Vec<_> = tuple_data
                    .iter()
                    .map(|td| crate::entry_type(&td.ty, td.close, td.ty.span()))
                    .collect();
                quote::quote_spanned! { ident_span=>
                    #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`")]
                    #[doc(hidden)]
                    #ident(#(#entry_types),*)
                }
            }
            Some(VariantData::Struct(fields)) => {
                let field_defs: Vec<_> =
                    fields.iter().filter_map(|f| f.entry_field(true)).collect();
                quote::quote_spanned! { ident_span=>
                    #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`")]
                    #[doc(hidden)]
                    #ident { #(#field_defs),* }
                }
            }
        }
    }
}

fn parse_variant_data(fields: &syn::Fields) -> Result<Option<VariantData>> {
    match fields {
        syn::Fields::Unit => Ok(None),
        syn::Fields::Unnamed(fields) => {
            let tuple_data: Result<Vec<_>> = fields
                .unnamed
                .iter()
                .map(|field| {
                    let raw_attrs = RawMetricsFieldAttrs::from_field(field)?;
                    let attrs = raw_attrs.validate()?;

                    match &attrs.kind {
                        MetricsFieldKind::Flatten { .. }
                        | MetricsFieldKind::FlattenEntry(_)
                        | MetricsFieldKind::Ignore(_) => {}
                        MetricsFieldKind::Timestamp(_) | MetricsFieldKind::Field { .. } => {
                            return Err(syn::Error::new_spanned(
                                field,
                                "tuple variant fields must use #[metrics(flatten)], #[metrics(flatten_entry)], or #[metrics(ignore)]",
                            ));
                        }
                    };

                    Ok(TupleData {
                        ty: field.ty.clone(),
                        kind: attrs.kind,
                        close: attrs.close,
                    })
                })
                .collect();

            Ok(Some(VariantData::Tuple(tuple_data?)))
        }
        syn::Fields::Named(fields) => {
            let parsed_fields = parse_metric_fields(&fields.named)?;
            Ok(Some(VariantData::Struct(parsed_fields)))
        }
    }
}

pub(crate) fn parse_enum_variants(
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
    mode: VariantMode,
) -> Result<Vec<MetricsVariant>> {
    // Value enums must have at least one variant, since otherwise what would its value type
    // return
    if mode == VariantMode::ValueString && variants.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "value enums must have at least one variant",
        ));
    }

    let mut parsed_variants = vec![];
    let mut errors = darling::Error::accumulator();

    for variant in variants {
        // Check for value enum with data first, before parsing
        if mode == VariantMode::ValueString && !variant.fields.is_empty() {
            errors.push(
                darling::Error::custom("value(string) enum variants may not contain data. Use #[metrics(tag(name=\"MyField\"))] to inject a value(string) field into an entry enum.")
                    .with_span(variant),
            );
            continue;
        }

        let data = if mode == VariantMode::SkipAttributeParsing {
            None
        } else {
            match parse_variant_data(&variant.fields) {
                Ok(d) => d,
                Err(e) => {
                    errors.push(darling::Error::from(e));
                    None
                }
            }
        };

        let attrs = if mode != VariantMode::SkipAttributeParsing {
            match errors.handle(RawMetricsVariantAttrs::from_variant(variant)) {
                Some(attrs) => attrs.validate(mode)?,
                None => {
                    continue;
                }
            }
        } else {
            MetricsVariantAttrs::default()
        };

        parsed_variants.push(MetricsVariant {
            ident: variant.ident.clone(),
            external_attrs: clean_attrs(&variant.attrs),
            attrs,
            data,
        });
    }

    errors.finish()?;

    // Empty enums are not allowed
    if parsed_variants.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "enums must have at least one variant",
        ));
    }

    Ok(parsed_variants)
}

pub(crate) fn generate_metrics_for_enum(
    root_attrs: RootAttributes,
    input: &syn::DeriveInput,
    variants: &[MetricsVariant],
) -> Result<Ts2> {
    let enum_name = &input.ident;
    let entry_name = if root_attrs.mode == MetricMode::ValueString {
        quote::format_ident!("{}Value", enum_name)
    } else {
        quote::format_ident!("{}Entry", enum_name)
    };
    let guard_name = quote::format_ident!("{}Guard", enum_name);
    let handle_name = quote::format_ident!("{}Handle", enum_name);

    let base_enum = generate_base_enum(
        enum_name,
        &input.vis,
        &input.generics,
        &clean_attrs(&input.attrs),
        variants,
    );
    let warnings = root_attrs.warnings();

    let entry_enum = generate_entry_enum(&entry_name, &input.generics, variants)?;

    let inner_impl = match root_attrs.mode {
        MetricMode::ValueString => value_impl::generate_value_impl_for_enum(
            &root_attrs,
            &entry_name,
            &input.generics,
            variants,
        ),
        _ => crate::entry_impl::generate_enum_entry_impl(
            &entry_name,
            &input.generics,
            variants,
            &root_attrs,
        ),
    };

    let close_value_impl = match root_attrs.mode {
        MetricMode::ValueString => {
            let variants_map = variants.iter().map(|variant| {
                let variant_ident = &variant.ident;
                quote::quote_spanned!(variant.ident.span()=> #enum_name::#variant_ident => #entry_name::#variant_ident)
            });
            let variants_map = quote!(#[allow(deprecated)] match self { #(#variants_map),* });
            crate::generate_close_value_impls(
                &root_attrs,
                enum_name,
                &entry_name,
                &input.generics,
                variants_map,
            )
        }
        _ => generate_close_value_impl_for_enum(
            enum_name,
            &entry_name,
            &input.generics,
            variants,
            &root_attrs,
        ),
    };

    let from_and_sample_group =
        generate_from_and_sample_group_for_enum(enum_name, &input.generics, variants, &root_attrs);

    let vis = &input.vis;

    let root_entry_specifics = match root_attrs.mode {
        MetricMode::RootEntry => {
            let on_drop_wrapper = generate_on_drop_wrapper(
                vis,
                &guard_name,
                enum_name,
                &entry_name,
                &handle_name,
                &input.generics,
            );
            quote! {
                #on_drop_wrapper
            }
        }
        MetricMode::Subfield
        | MetricMode::SubfieldOwned
        | MetricMode::ValueString
        | MetricMode::Value => {
            quote! {}
        }
    };

    Ok(quote! {
        #base_enum
        #entry_enum
        #inner_impl
        #close_value_impl
        #from_and_sample_group
        #root_entry_specifics
        #warnings
    })
}

pub(crate) fn generate_base_enum(
    name: &Ident,
    vis: &Visibility,
    generics: &Generics,
    attrs: &[Attribute],
    variants: &[MetricsVariant],
) -> Ts2 {
    let variants = variants.iter().map(|f| f.core_variant());
    let data = quote! {
        #(#variants),*
    };
    quote! {
        #(#attrs)*
        #vis enum #name #generics { #data }
    }
}

fn generate_entry_enum(
    name: &Ident,
    generics: &Generics,
    variants: &[MetricsVariant],
) -> Result<Ts2> {
    let variants = variants.iter().map(|variant| variant.entry_variant());
    let data = quote! {
        #(#variants,)*
    };
    Ok(quote! {
        #[doc(hidden)]
        pub enum #name #generics {
            #data
        }
    })
}

fn generate_close_value_impl_for_enum(
    enum_name: &Ident,
    entry_name: &Ident,
    generics: &Generics,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let match_arms = variants.iter().map(|variant| {
        let variant_ident = &variant.ident;

        match &variant.data {
            None => {
                // Unit variant: Enum::Variant => Entry::Variant
                quote::quote_spanned!(variant.ident.span()=>
                    #enum_name::#variant_ident => #entry_name::#variant_ident
                )
            }
            Some(VariantData::Tuple(tuple_data)) => {
                // Tuple variant: Enum::Variant(v1, v2, ...) => Entry::Variant(close_expr1, close_expr2, ...)
                let (bindings, close_exprs): (Vec<_>, Vec<_>) = tuple_data
                    .iter()
                    .enumerate()
                    .map(|(i, td)| {
                        let binding = quote::format_ident!("v{}", i);
                        let close_expr = if td.close {
                            quote::quote_spanned!(variant.ident.span()=>
                                ::metrique::CloseValue::close(#binding)
                            )
                        } else {
                            quote::quote_spanned!(variant.ident.span()=> #binding)
                        };
                        (binding, close_expr)
                    })
                    .unzip();
                quote::quote_spanned!(variant.ident.span()=>
                    #enum_name::#variant_ident(#(#bindings),*) => #entry_name::#variant_ident(#(#close_exprs),*)
                )
            }
            Some(VariantData::Struct(fields)) => {
                // Struct variant: Enum::Variant { fields } => Entry::Variant { closed_fields }
                let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();
                let closed_fields: Vec<_> = fields
                    .iter()
                    .map(|f| {
                        let ident: &Ts2 = &f.ident;
                        f.close_field_expr(quote::quote_spanned! {f.span=> #ident })
                    })
                    .collect();
                quote::quote_spanned!(variant.ident.span()=>
                    #enum_name::#variant_ident { #(#field_names),* } => #entry_name::#variant_ident { #(#closed_fields),* }
                )
            }
        }
    });

    let match_expr = quote!(#[allow(deprecated)] match self { #(#match_arms),* });

    crate::generate_close_value_impls(root_attrs, enum_name, entry_name, generics, match_expr)
}

pub(crate) fn generate_from_and_sample_group_for_enum(
    enum_name: &Ident,
    generics: &Generics,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let variants_and_strings = variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        let metric_name = crate::inflect::inflect_no_prefix(root_attrs, variant);
        let pattern = match &variant.data {
            None => quote::quote_spanned!(variant.ident.span()=> #enum_name::#variant_ident),
            Some(VariantData::Tuple(tuple_data)) => {
                let wildcards = (0..tuple_data.len()).map(|_| quote!(_));
                quote::quote_spanned!(variant.ident.span()=> #enum_name::#variant_ident(#(#wildcards),*))
            }
            Some(VariantData::Struct(_)) => {
                quote::quote_spanned!(variant.ident.span()=> #enum_name::#variant_ident { .. })
            }
        };
        quote::quote_spanned!(variant.ident.span()=> #pattern => #metric_name)
    });

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    quote! {
        impl #impl_generics ::std::convert::From<&'_ #enum_name #ty_generics> for &'static str #where_clause {
            fn from(value: &#enum_name #ty_generics) -> Self {
                #[allow(deprecated)] match value {
                    #(#variants_and_strings),*
                }
            }
        }
        impl #impl_generics ::std::convert::From<#enum_name #ty_generics> for &'static str #where_clause {
            fn from(value: #enum_name #ty_generics) -> Self {
                <&str as ::std::convert::From<&_>>::from(&value)
            }
        }
        impl #impl_generics ::metrique::writer::core::SampleGroup for #enum_name #ty_generics #where_clause {
            fn as_sample_group(&self) -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(::std::convert::Into::<&str>::into(self))
            }
        }
    }
}
