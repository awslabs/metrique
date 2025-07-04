// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use proc_macro2::TokenStream as Ts2;
use quote::{quote, quote_spanned};
use syn::Ident;

use crate::{MetricsField, MetricsFieldAttrs, NameStyle, RootAttributes};

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
            // The fields of the entry are all marked "deprecated" so that people don't use them directly.
            #[expect(deprecated)]
            impl<NS: ::metrique::NameStyle> ::metrique::InflectableEntry<NS> for #entry_name {
                fn write<'a>(&'a self, writer: &mut impl ::metrique::__writer::EntryWriter<'a>) {
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
            ::metrique::__writer::Entry::write(&self.#field_ident, writer);
        });
    }

    for field in fields {
        let field_ident = &field.ident;
        let field_span = field_ident.span();
        let ns = make_ns(root_attrs.rename_all, field_span);

        match &field.attrs {
            MetricsFieldAttrs::Timestamp(span) => {
                writes.push(quote_spanned! {*span=>
                    #[allow(clippy::useless_conversion)]
                    {
                        ::metrique::__writer::EntryWriter::timestamp(writer, (self.#field_ident).into());
                    }
                });
            }
            MetricsFieldAttrs::FlattenEntry(span) => {
                writes.push(quote_spanned! {*span=>
                    ::metrique::__writer::Entry::write(&self.#field_ident, writer);
                });
            }
            MetricsFieldAttrs::Flatten(span) => {
                writes.push(quote_spanned! {*span=>
                    ::metrique::InflectableEntry::<#ns>::write(&self.#field_ident, writer);
                });
            }
            MetricsFieldAttrs::Ignore(_) => {
                continue;
            }
            MetricsFieldAttrs::Field { format, .. } => {
                let name_ident =
                    metric_name(field, &NameStyle::Preserve, root_attrs.prefix.as_deref());
                let name_pascal =
                    metric_name(field, &NameStyle::PascalCase, root_attrs.prefix.as_deref());
                let name_snake =
                    metric_name(field, &NameStyle::SnakeCase, root_attrs.prefix.as_deref());
                let name_kebab =
                    metric_name(field, &NameStyle::KebabCase, root_attrs.prefix.as_deref());
                let formatted = |field| {
                    if let Some(format) = format {
                        quote_spanned! { field_span=> &::metrique::format::FormattedValue::<_, #format>::new(#field)}
                    } else {
                        field
                    }
                };
                let value = formatted(quote! { &self.#field_ident });
                writes.push(quote_spanned! {field_span=>
                    ::metrique::__writer::EntryWriter::value(writer,
                        <#ns as ::metrique::NameStyle>::inflect_name(#name_ident, #name_pascal, #name_snake, #name_kebab)
                        , #value);
                });
            }
        }
    }

    writes
}

fn metric_name(field: &MetricsField, name_style: &NameStyle, prefix: Option<&str>) -> String {
    let prefix = prefix.unwrap_or_default();

    if let MetricsFieldAttrs::Field {
        name: Some(name), ..
    } = &field.attrs
    {
        return name.to_owned();
    };
    let base = &field.ident.to_string();
    let prefixed_base = format!("{prefix}{base}");

    name_style.apply(&prefixed_base)
}

fn generate_sample_group_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Ts2 {
    let mut sample_group_fields = Vec::new();

    for field in fields {
        if let MetricsFieldAttrs::Ignore(_) = field.attrs {
            continue;
        }

        let field_ident = &field.ident;

        match &field.attrs {
            MetricsFieldAttrs::Flatten(span) => {
                let ns = make_ns(root_attrs.rename_all, field.ident.span());
                sample_group_fields.push(quote_spanned! {*span=>
                    ::metrique::InflectableEntry::<#ns>::sample_group(&self.#field_ident)
                });
            }
            _ => {
                // TODO: support sample_group
            }
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
