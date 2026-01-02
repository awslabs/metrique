// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use darling::FromVariant;
use proc_macro2::TokenStream as Ts2;
use quote::quote;
use syn::{Attribute, Generics, Ident, Result, Visibility};

use crate::{RootAttributes, clean_attrs, value_impl};

#[derive(Debug, FromVariant)]
#[darling(attributes(metrics))]
struct RawMetricsVariantAttrs {
    #[darling(default)]
    name: Option<crate::SpannedKv<String>>,
}

impl RawMetricsVariantAttrs {
    fn validate(self) -> darling::Result<MetricsVariantAttrs> {
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
}

impl MetricsVariant {
    pub(crate) fn core_variant(&self) -> Ts2 {
        let MetricsVariant {
            ref external_attrs,
            ref ident,
            ..
        } = *self;
        quote! { #(#external_attrs)* #ident }
    }

    pub(crate) fn entry_variant(&self) -> Ts2 {
        let ident_span = self.ident.span();
        let ident = &self.ident;
        quote::quote_spanned! { ident_span=>
            #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`")]
            #[doc(hidden)]
            #ident
        }
    }
}

pub(crate) fn parse_enum_variants(
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
    parse_attrs: bool,
) -> Result<Vec<MetricsVariant>> {
    let mut parsed_variants = vec![];
    let mut errors = darling::Error::accumulator();

    for variant in variants {
        if !variant.fields.is_empty() {
            return Err(syn::Error::new_spanned(
                variant,
                "variants with fields are not supported",
            ));
        }

        let attrs = if parse_attrs {
            match errors.handle(RawMetricsVariantAttrs::from_variant(variant)) {
                Some(attrs) => attrs.validate()?,
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
        });
    }

    errors.finish()?;

    Ok(parsed_variants)
}

pub(crate) fn generate_metrics_for_enum(
    root_attrs: RootAttributes,
    input: &syn::DeriveInput,
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
) -> Result<Ts2> {
    let enum_name = &input.ident;
    let parsed_variants = parse_enum_variants(variants, true)?;
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
        crate::generate_close_value_impls(&root_attrs, enum_name, &value_name, variants_map);

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
