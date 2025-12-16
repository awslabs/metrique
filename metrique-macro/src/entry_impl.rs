// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use proc_macro2::TokenStream as Ts2;
use quote::{format_ident, quote, quote_spanned};
use syn::Ident;

use crate::{
    MetricsField, MetricsFieldKind, NameStyle, Prefix, RootAttributes, inflect::metric_name,
    value_impl::format_value,
};

/// Generate the implementation of the Entry trait directly instead of using derive(Entry).
/// This gives us more control over the generated code and improves compile-time errors.
pub fn generate_entry_impl(
    entry_name: &Ident,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let writes = generate_write_statements(fields, root_attrs);
    let sample_groups = generate_sample_group_statements(fields, root_attrs);
    // we generate one entry impl for each namestyle. This will then allow the parent to
    // transitively set the namestyle
    quote! {
        const _: () = {
            #[expect(deprecated)]
            impl<NS: ::metrique::NameStyle> ::metrique::InflectableEntry<NS> for #entry_name {
                fn write<'a>(&'a self, writer: &mut impl ::metrique::writer::EntryWriter<'a>) {
                    #(#writes)*
                }

                fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                    #sample_groups
                }
            }
        };
    }
}

fn make_ns(ns: NameStyle, span: proc_macro2::Span) -> Ts2 {
    match ns {
        NameStyle::PascalCase => quote_spanned! {span=> NS::PascalCase },
        NameStyle::SnakeCase => quote_spanned! {span=> NS::SnakeCase },
        NameStyle::KebabCase => quote_spanned! {span=> NS::KebabCase },
        NameStyle::Preserve => quote_spanned! {span=> NS },
    }
}

fn generate_write_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Vec<Ts2> {
    let mut writes = Vec::new();

    for field_ident in root_attrs.configuration_field_names() {
        writes.push(quote! {
            ::metrique::writer::Entry::write(&self.#field_ident, writer);
        });
    }

    for field in fields {
        let field_ident = &field.ident;
        let field_span = field.span;
        let ns = make_ns(root_attrs.rename_all, field_span);

        match &field.attrs.kind {
            MetricsFieldKind::Timestamp(span) => {
                writes.push(quote_spanned! {*span=>
                    #[allow(clippy::useless_conversion)]
                    {
                        ::metrique::writer::EntryWriter::timestamp(writer, (self.#field_ident).into());
                    }
                });
            }
            MetricsFieldKind::FlattenEntry(span) => {
                writes.push(quote_spanned! {*span=>
                    ::metrique::writer::Entry::write(&self.#field_ident, writer);
                });
            }
            MetricsFieldKind::Flatten { span, prefix } => {
                let (extra, ns) = match prefix {
                    None => (quote!(), ns),
                    Some(Prefix::Inflectable { prefix }) => append_prefix_to_ns(
                        &ns,
                        make_inflect(
                            &ns,
                            format_ident!("InflectAffix", span = field_span),
                            |style| style.apply_prefix(prefix),
                            field,
                        ),
                        field,
                    ),
                    Some(Prefix::Exact(exact_prefix)) => append_prefix_to_ns(
                        &ns,
                        make_const_str_noinflect(exact_prefix.clone(), field),
                        field,
                    ),
                };
                writes.push(quote_spanned! {*span=>
                    #extra
                    ::metrique::InflectableEntry::<#ns>::write(&self.#field_ident, writer);
                });
            }
            MetricsFieldKind::Ignore(_) => {
                continue;
            }
            MetricsFieldKind::Field { format, .. } => {
                let (extra, name) = make_inflect_metric_name(root_attrs, field);
                let value = format_value(format, field_span, quote! { &self.#field_ident });
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

fn append_prefix_to_ns(ns: &Ts2, (extra, prefix): (Ts2, Ts2), field: &MetricsField) -> (Ts2, Ts2) {
    (
        extra,
        quote_spanned! {field.span=>
            <#ns as ::metrique::NameStyle>::AppendPrefix<#prefix>
        },
    )
}

fn make_inflect_metric_name(root_attrs: &RootAttributes, field: &MetricsField) -> (Ts2, Ts2) {
    make_inflect(
        &make_ns(root_attrs.rename_all, field.span),
        format_ident!("Inflect", span = field.span),
        |style| metric_name(root_attrs, style, field),
        field,
    )
}

fn make_inflect(
    ns: &Ts2,
    inflect: syn::Ident,
    mut name: impl FnMut(NameStyle) -> String,
    field: &MetricsField,
) -> (Ts2, Ts2) {
    let name_ident = const_str_struct_name(NameStyle::Preserve, field);
    let name_kebab = const_str_struct_name(NameStyle::KebabCase, field);
    let name_pascal = const_str_struct_name(NameStyle::PascalCase, field);
    let name_snake = const_str_struct_name(NameStyle::SnakeCase, field);

    let extra_preserve = const_str(&name_ident, &name(NameStyle::Preserve));
    let extra_kebab = const_str(&name_kebab, &name(NameStyle::KebabCase));
    let extra_pascal = const_str(&name_pascal, &name(NameStyle::PascalCase));
    let extra_snake = const_str(&name_snake, &name(NameStyle::SnakeCase));

    (
        quote!(
            #extra_preserve
            #extra_kebab
            #extra_pascal
            #extra_snake
        ),
        quote!(
            <#ns as ::metrique::NameStyle>::#inflect<#name_ident, #name_pascal, #name_snake, #name_kebab>
        ),
    )
}

fn make_const_str_noinflect(name: String, field: &MetricsField) -> (Ts2, Ts2) {
    let name_ident = const_str_struct_name(NameStyle::Preserve, field);

    let extra = const_str(&name_ident, &name);

    (extra, quote! { #name_ident })
}

pub fn const_str(ident: &syn::Ident, value: &str) -> Ts2 {
    quote_spanned! {ident.span()=>
        #[allow(non_camel_case_types)]
        struct #ident;
        impl ::metrique::concat::ConstStr for #ident {
            const VAL: &'static str = #value;
        }
    }
}

fn const_str_struct_name(name_style: NameStyle, field: &MetricsField) -> syn::Ident {
    format_ident!(
        "{}{}",
        field.ident.to_string(),
        name_style.to_word(),
        span = field.span
    )
}

fn generate_sample_group_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Ts2 {
    let mut sample_group_fields = Vec::new();

    for field in fields {
        if let MetricsFieldKind::Ignore(_) = field.attrs.kind {
            continue;
        }

        let field_ident = &field.ident;

        match &field.attrs.kind {
            MetricsFieldKind::Flatten { span, prefix: _ } => {
                let ns = make_ns(root_attrs.rename_all, field.span);
                sample_group_fields.push(quote_spanned! {*span=>
                    ::metrique::InflectableEntry::<#ns>::sample_group(&self.#field_ident)
                });
            }
            MetricsFieldKind::FlattenEntry(span) => {
                sample_group_fields.push(quote_spanned! {*span=>
                    ::metrique::writer::Entry::sample_group(&self.#field_ident)
                });
            }
            MetricsFieldKind::Field {
                sample_group: Some(span),
                ..
            } => {
                let (extra, name) = make_inflect_metric_name(root_attrs, field);
                sample_group_fields.push(quote_spanned! {*span=>
                    {
                        #extra
                        ::std::iter::once((
                            ::metrique::concat::const_str_value::<#name>(),
                            ::metrique::writer::core::SampleGroup::as_sample_group(&self.#field_ident)
                        ))
                    }
                });
            }
            // these don't have sample groups
            MetricsFieldKind::Field {
                sample_group: None, ..
            }
            | MetricsFieldKind::Ignore { .. }
            | MetricsFieldKind::Timestamp { .. } => {}
        }
    }

    // If we have sample group fields, chain them together
    if !sample_group_fields.is_empty() {
        // Create a binary tree of chain calls to avoid deep nesting
        make_binary_tree_chain(sample_group_fields)
    } else {
        // Return empty iterator if no sample groups
        quote! { ::std::iter::empty() }
    }
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
