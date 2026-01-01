// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use darling::FromField;
use darling::FromVariant;
use proc_macro2::TokenStream as Ts2;
use quote::quote;
use syn::{Attribute, Generics, Ident, Result, Visibility, spanned::Spanned};

use crate::TupleData;
use crate::{
    MetricsField, MetricsFieldKind, RawMetricsFieldAttrs, RootAttributes, SpannedKv, clean_attrs,
    generate_close_value_impls, parse_metric_fields, value_impl,
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
                let entry_types = tuple_data
                    .iter()
                    .map(|td| crate::entry_type(&td.ty, td.close, td.ty.span()));
                quote::quote_spanned! { ident_span=>
                    #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`")]
                    #[doc(hidden)]
                    #ident(#(#entry_types),*)
                }
            }
            Some(VariantData::Struct(fields)) => {
                let field_defs = fields.iter().filter_map(|f| f.entry_field(true));
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
                        _kind: attrs.kind,
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
                darling::Error::custom("value(string) enum variants may not contain data")
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

    // Entry enums must have ALL data variants (no unit variants allowed)
    if mode == VariantMode::Entry {
        for variant in &parsed_variants {
            if variant.data.is_none() {
                return Err(syn::Error::new_spanned(
                    &variant.ident,
                    "entry enums cannot have unit variants; use #[metrics(value(string))] for unit-only enums",
                ));
            }
        }
    }

    Ok(parsed_variants)
}

pub(crate) fn generate_metrics_for_enum(
    root_attrs: RootAttributes,
    input: &syn::DeriveInput,
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
) -> Result<Ts2> {
    let enum_name = &input.ident;
    let parsed_variants = parse_enum_variants(variants, VariantMode::ValueString)?;
    let value_name = quote::format_ident!("{}Value", enum_name);

    let base_enum = generate_base_enum(
        enum_name,
        &input.vis,
        &input.generics,
        &input.attrs,
        &parsed_variants,
    );
    let warnings = root_attrs.warnings();

    let value_enum =
        generate_value_enum(&value_name, &input.generics, &parsed_variants, &root_attrs)?;

    let value_impl =
        value_impl::generate_value_impl_for_enum(&root_attrs, &value_name, &parsed_variants);

    let variants_map = parsed_variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        quote::quote_spanned!(variant.ident.span()=> #enum_name::#variant_ident => #value_name::#variant_ident)
    });
    let variants_map = quote!(#[allow(deprecated)] match self { #(#variants_map),* });

    let close_value_impl =
        generate_close_value_impls(&root_attrs, enum_name, &value_name, variants_map);

    Ok(quote! {
        #base_enum
        #value_enum
        #value_impl
        #close_value_impl
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

fn generate_value_enum(
    name: &Ident,
    _generics: &Generics,
    variants: &[MetricsVariant],
    _root_attrs: &RootAttributes,
) -> Result<Ts2> {
    let variants = variants.iter().map(|variant| variant.entry_variant());
    let data = quote! {
        #(#variants,)*
    };
    Ok(quote! {
        #[doc(hidden)]
        pub enum #name {
            #data
        }
    })
}
