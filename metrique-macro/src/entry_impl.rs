// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module generates the implementation of the Entry trait for non-value structs and enums.
//! This gives us more control over the generated code and improves compile-time errors.

use proc_macro2::TokenStream as Ts2;
use quote::{format_ident, quote, quote_spanned};
use syn::Ident;

use crate::{
    MetricsField, MetricsFieldKind, NameStyle, Prefix, RootAttributes, inflect::metric_name,
};

mod enum_impl;
mod struct_impl;

pub(crate) use enum_impl::generate_enum_entry_impl;
pub(crate) use struct_impl::generate_struct_entry_impl;

fn make_ns(ns: NameStyle, span: proc_macro2::Span) -> Ts2 {
    match ns {
        NameStyle::PascalCase => quote_spanned! {span=> NS::PascalCase },
        NameStyle::SnakeCase => quote_spanned! {span=> NS::SnakeCase },
        NameStyle::KebabCase => quote_spanned! {span=> NS::KebabCase },
        NameStyle::Preserve => quote_spanned! {span=> NS },
    }
}

// Shared helpers for both struct and enum implementations

/// Generate a ConstStr struct with the given identifier and value.
/// Used to create compile-time constant strings for metric names and prefixes.
fn const_str(ident: &syn::Ident, value: &str) -> Ts2 {
    quote_spanned! {ident.span()=>
        #[allow(non_camel_case_types)]
        struct #ident;
        impl ::metrique::concat::ConstStr for #ident {
            const VAL: &'static str = #value;
        }
    }
}

/// Generate 4 ConstStr structs (one per naming style) and build an Inflect namespace type.
/// The `name_fn` callback computes the string value for each style.
/// Returns (extra_code, inflected_type).
fn make_inflect(
    ns: &Ts2,
    inflect_method: syn::Ident,
    base_name: &str,
    span: proc_macro2::Span,
    mut name_fn: impl FnMut(NameStyle) -> String,
) -> (Ts2, Ts2) {
    let name_ident = format_ident!(
        "{}{}",
        base_name,
        NameStyle::Preserve.to_word(),
        span = span
    );
    let name_kebab = format_ident!(
        "{}{}",
        base_name,
        NameStyle::KebabCase.to_word(),
        span = span
    );
    let name_pascal = format_ident!(
        "{}{}",
        base_name,
        NameStyle::PascalCase.to_word(),
        span = span
    );
    let name_snake = format_ident!(
        "{}{}",
        base_name,
        NameStyle::SnakeCase.to_word(),
        span = span
    );

    let extra_preserve = const_str(&name_ident, &name_fn(NameStyle::Preserve));
    let extra_kebab = const_str(&name_kebab, &name_fn(NameStyle::KebabCase));
    let extra_pascal = const_str(&name_pascal, &name_fn(NameStyle::PascalCase));
    let extra_snake = const_str(&name_snake, &name_fn(NameStyle::SnakeCase));

    let extra = quote!(
        #extra_preserve
        #extra_kebab
        #extra_pascal
        #extra_snake
    );

    let inflected_type = quote!(
        <#ns as ::metrique::NameStyle>::#inflect_method<#name_ident, #name_pascal, #name_snake, #name_kebab>
    );

    (extra, inflected_type)
}

/// Generate an inflectable prefix that adapts to the namespace style.
/// Creates 4 ConstStr structs (preserve, pascal, snake, kebab) and returns
/// a namespace type that selects the appropriate variant via InflectAffix.
/// Returns (extra_code, namespace_with_prefix).
fn make_inflect_prefix(
    ns: &Ts2,
    prefix: &str,
    base_name: &str,
    span: proc_macro2::Span,
) -> (Ts2, Ts2) {
    let (extra, inflected) = make_inflect(
        ns,
        format_ident!("InflectAffix", span = span),
        &format!("{}Prefix", base_name),
        span,
        |style| style.apply_prefix(prefix),
    );

    let ns_with_prefix = quote!(
        <#ns as ::metrique::NameStyle>::AppendPrefix<#inflected>
    );

    (extra, ns_with_prefix)
}

/// Generate an exact (non-inflectable) prefix that never changes.
/// Creates 1 ConstStr struct and returns a namespace type with the prefix applied.
/// Returns (extra_code, namespace_with_prefix).
fn make_exact_prefix(
    ns: &Ts2,
    exact_prefix: &str,
    base_name: &str,
    span: proc_macro2::Span,
) -> (Ts2, Ts2) {
    let prefix_ident = format_ident!("{}Preserve", base_name, span = span);
    let extra = const_str(&prefix_ident, exact_prefix);
    let ns_with_prefix = quote!(
        <#ns as ::metrique::NameStyle>::AppendPrefix<#prefix_ident>
    );
    (extra, ns_with_prefix)
}

fn generate_field_writes(
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
    field_access: impl Fn(&Ts2) -> Ts2,
) -> Vec<Ts2> {
    let mut writes = Vec::new();

    for field in fields {
        let field_span = field.span;
        let ns = make_ns(root_attrs.rename_all, field_span);

        match &field.attrs.kind {
            MetricsFieldKind::Timestamp(span) => {
                let field_access = field_access(&field.ident);
                writes.push(quote_spanned! {*span=>
                    #[allow(clippy::useless_conversion)]
                    {
                        ::metrique::writer::EntryWriter::timestamp(writer, (*#field_access).into());
                    }
                });
            }
            MetricsFieldKind::FlattenEntry(span) => {
                let field_access = field_access(&field.ident);
                writes.push(quote_spanned! {*span=>
                    ::metrique::writer::Entry::write(#field_access, writer);
                });
            }
            MetricsFieldKind::Flatten { span, prefix } => {
                let (extra, ns) = match prefix {
                    None => (quote!(), ns),
                    Some(Prefix::Inflectable { prefix }) => {
                        make_inflect_prefix(&ns, prefix, &field.ident.to_string(), field_span)
                    }
                    Some(Prefix::Exact(exact_prefix)) => {
                        make_exact_prefix(&ns, exact_prefix, &field.ident.to_string(), field_span)
                    }
                };
                let field_access = field_access(&field.ident);
                writes.push(quote_spanned! {*span=>
                    #extra
                    ::metrique::InflectableEntry::<#ns>::write(#field_access, writer);
                });
            }
            MetricsFieldKind::Ignore(_) => {
                continue;
            }
            MetricsFieldKind::Field { format, .. } => {
                let (extra, name) = make_inflect(
                    &ns,
                    format_ident!("Inflect", span = field_span),
                    &field.ident.to_string(),
                    field_span,
                    |style| crate::inflect::metric_name(root_attrs, style, field),
                );
                let field_access = field_access(&field.ident);
                let value = crate::value_impl::format_value(format, field_span, field_access);
                writes.push(quote_spanned! {field_span=>
                    ::metrique::writer::EntryWriter::value(writer,
                        {
                            #extra
                            ::metrique::concat::const_str_value::<#name>()
                        }
                        , #value);
                });
            }
        }
    }

    writes
}

/// Return an iterator that chains the iterators in `iterators`.
///
/// This calls `chain` in a binary tree fashion to avoid problems with the recursion limit,
/// e.g. `I1.chain(I2).chain(I3.chain(I4))`
fn make_binary_tree_chain(iterators: Vec<Ts2>) -> Ts2 {
    if iterators.is_empty() {
        return quote! { ::std::iter::empty() };
    }

    if iterators.len() == 1 {
        return iterators[0].clone();
    }

    // Split the iterators in half and recursively build the tree
    let mid = iterators.len() / 2;
    let left = make_binary_tree_chain(iterators[..mid].to_vec());
    let right = make_binary_tree_chain(iterators[mid..].to_vec());

    quote! { #left.chain(#right) }
}

fn make_inflect_metric_name(root_attrs: &RootAttributes, field: &MetricsField) -> (Ts2, Ts2) {
    make_inflect(
        &make_ns(root_attrs.rename_all, field.span),
        format_ident!("Inflect", span = field.span),
        &field.ident.to_string(),
        field.span,
        |style| metric_name(root_attrs, style, field),
    )
}

/// Collect sample group iterators from a field, returning (field_ident, iterator_expr) for fields that have sample groups
/// The `field_access` closure determines how to access the field (e.g., `#field_ident` or `&self.#field_ident`)
fn collect_field_sample_group<'a>(
    field: &'a MetricsField,
    root_attrs: &RootAttributes,
    field_access: impl FnOnce(&Ts2) -> Ts2,
) -> Option<(&'a Ts2, Ts2)> {
    let field_ident = &field.ident;
    match &field.attrs.kind {
        MetricsFieldKind::Flatten { span, .. } => {
            let ns = make_ns(root_attrs.rename_all, field.span);
            let access = field_access(field_ident);
            Some((
                field_ident,
                quote_spanned!(*span=>
                    ::metrique::InflectableEntry::<#ns>::sample_group(#access)
                ),
            ))
        }
        MetricsFieldKind::FlattenEntry(span) => {
            let access = field_access(field_ident);
            Some((
                field_ident,
                quote_spanned!(*span=>
                    ::metrique::writer::Entry::sample_group(#access)
                ),
            ))
        }
        MetricsFieldKind::Field {
            sample_group: Some(span),
            ..
        } => {
            let (extra, name) = make_inflect_metric_name(root_attrs, field);
            let access = field_access(field_ident);
            Some((
                field_ident,
                quote_spanned!(*span=>
                    {
                        #extra
                        ::std::iter::once((
                            ::metrique::concat::const_str_value::<#name>(),
                            ::metrique::writer::core::SampleGroup::as_sample_group(#access)
                        ))
                    }
                ),
            ))
        }
        MetricsFieldKind::Field {
            sample_group: None, ..
        }
        | MetricsFieldKind::Ignore(_)
        | MetricsFieldKind::Timestamp(_) => None,
    }
}
