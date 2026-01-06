// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use proc_macro2::TokenStream as Ts2;
use quote::{format_ident, quote};
use syn::{
    Attribute, DeriveInput, FieldsNamed, FieldsUnnamed, Generics, Ident, Result, Visibility,
};

use crate::{
    MetricMode, MetricsField, MetricsFieldKind, RootAttributes, clean_attrs, entry_impl,
    generate_close_value_impls, generate_on_drop_wrapper, parse_metric_fields, value_impl,
};

pub(crate) fn generate_metrics_for_struct(
    root_attributes: RootAttributes,
    input: &DeriveInput,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> Result<Ts2> {
    let struct_name = &input.ident;
    let entry_name = if root_attributes.mode == MetricMode::Value {
        format_ident!("{}Value", struct_name)
    } else {
        format_ident!("{}Entry", struct_name)
    };
    let guard_name = format_ident!("{}Guard", struct_name);
    let handle_name = format_ident!("{}Handle", struct_name);

    let parsed_fields = parse_metric_fields(fields)?;

    let base_struct = generate_base_struct(
        struct_name,
        &input.vis,
        &input.generics,
        &clean_attrs(&input.attrs),
        &parsed_fields,
    )?;
    let warnings = root_attributes.warnings();

    let entry_struct = generate_entry_struct(
        &entry_name,
        &input.vis,
        &input.generics,
        &parsed_fields,
        &root_attributes,
    )?;

    let inner_impl = match root_attributes.mode {
        MetricMode::Value => {
            value_impl::validate_value_impl_for_struct(
                &root_attributes,
                &entry_name,
                &parsed_fields,
            )?;
            value_impl::generate_value_impl_for_struct(
                &root_attributes,
                &entry_name,
                &parsed_fields,
            )?
        }
        _ => entry_impl::generate_struct_entry_impl(&entry_name, &parsed_fields, &root_attributes),
    };

    let close_value_impl = generate_close_value_impls_for_struct(
        struct_name,
        &entry_name,
        &parsed_fields,
        &root_attributes,
    );
    let vis = &input.vis;

    let root_entry_specifics = match root_attributes.mode {
        MetricMode::RootEntry => {
            let on_drop_wrapper =
                generate_on_drop_wrapper(vis, &guard_name, struct_name, &entry_name, &handle_name);
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
        #base_struct
        #warnings
        #entry_struct
        #inner_impl
        #close_value_impl
        #root_entry_specifics
    })
}

fn generate_base_struct(
    name: &Ident,
    vis: &Visibility,
    generics: &Generics,
    attrs: &[Attribute],
    fields: &[MetricsField],
) -> Result<Ts2> {
    let has_named_fields = fields.iter().any(|f| f.name.is_some());
    let fields = fields.iter().map(|f| f.core_field(has_named_fields));
    let body = wrap_fields_into_struct_decl(has_named_fields, fields);

    Ok(quote! {
        #(#attrs)*
        #vis struct #name #generics #body
    })
}

fn wrap_fields_into_struct_decl(has_named_fields: bool, fields: impl Iterator<Item = Ts2>) -> Ts2 {
    if has_named_fields {
        quote! { { #(#fields,)* } }
    } else {
        quote! { ( #(#fields,)* ); }
    }
}

fn generate_entry_struct(
    name: &Ident,
    vis: &Visibility,
    _generics: &Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Result<Ts2> {
    let has_named_fields = fields.iter().any(|f| f.name.is_some());
    let config = root_attrs.configuration_fields();

    let fields = fields.iter().flat_map(|f| f.entry_field(has_named_fields));
    let body = wrap_fields_into_struct_decl(has_named_fields, config.into_iter().chain(fields));
    Ok(quote!(
        #[doc(hidden)]
        #vis struct #name #body
    ))
}

fn generate_close_value_impls_for_struct(
    metrics_struct: &Ident,
    entry: &Ident,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let fields = fields
        .iter()
        .filter(|f| !matches!(f.attrs.kind, MetricsFieldKind::Ignore(_)))
        .map(|f| f.close_value(root_attrs.ownership_kind()));
    let config: Vec<Ts2> = root_attrs.create_configuration();
    generate_close_value_impls(
        root_attrs,
        metrics_struct,
        entry,
        quote! {
            #[allow(deprecated)]
            #entry {
                #(#config,)*
                #(#fields,)*
            }
        },
    )
}

pub(crate) fn clean_base_struct(
    vis: &syn::Visibility,
    struct_name: &syn::Ident,
    generics: &syn::Generics,
    filtered_attrs: Vec<Attribute>,
    fields: &FieldsNamed,
) -> Ts2 {
    // Strip out `metrics` attribute
    let clean_fields = fields.named.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let field_vis = &field.vis;

        // Filter out metrics attributes
        let field_attrs = clean_attrs(&field.attrs);

        quote! {
            #(#field_attrs)*
            #field_vis #field_name: #field_type
        }
    });

    let expanded = quote! {
        #(#filtered_attrs)*
        #vis struct #struct_name #generics {
            #(#clean_fields),*
        }
    };

    expanded
}

pub(crate) fn clean_base_unnamed_struct(
    vis: &syn::Visibility,
    struct_name: &syn::Ident,
    generics: &syn::Generics,
    filtered_attrs: Vec<Attribute>,
    fields: &FieldsUnnamed,
) -> Ts2 {
    // Strip out `metrics` attribute
    let clean_fields = fields.unnamed.iter().map(|field| {
        let field_type = &field.ty;
        let field_vis = &field.vis;

        // Filter out metrics attributes
        let field_attrs = clean_attrs(&field.attrs);

        quote! {
            #(#field_attrs)*
            #field_vis #field_type
        }
    });

    let expanded = quote! {
        #(#filtered_attrs)*
        #vis struct #struct_name #generics (
            #(#clean_fields),*
        );
    };

    expanded
}
