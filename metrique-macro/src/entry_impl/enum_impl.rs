use super::*;
use crate::enums::{MetricsVariant, VariantData};

/// Build a struct variant pattern from field identifiers.
fn struct_pattern(
    entry_name: &Ident,
    variant_ident: &Ident,
    fields: &[&Ts2],
    exhaustive: bool,
) -> Ts2 {
    if exhaustive {
        quote!(#entry_name::#variant_ident { #(#fields),* })
    } else if !fields.is_empty() {
        quote!(#entry_name::#variant_ident { #(#fields),*, .. })
    } else {
        quote!(#entry_name::#variant_ident { .. })
    }
}

/// Build a tuple variant pattern from bindings.
fn tuple_pattern(entry_name: &Ident, variant_ident: &Ident, bindings: &[Ident]) -> Ts2 {
    quote!(#entry_name::#variant_ident(#(#bindings),*))
}

pub(crate) fn generate_enum_entry_impl(
    entry_name: &Ident,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let write_arms = generate_write_arms(entry_name, variants, root_attrs);
    let (iter_enum, sample_group_arms) =
        generate_sample_group_impl(entry_name, variants, root_attrs);

    quote! {
        const _: () = {
            #iter_enum

            #[expect(deprecated)]
            impl<NS: ::metrique::NameStyle> ::metrique::InflectableEntry<NS> for #entry_name {
                fn write<'a>(&'a self, writer: &mut impl ::metrique::writer::EntryWriter<'a>) {
                    #[allow(deprecated)]
                    match self {
                        #(#write_arms)*
                    }
                }

                fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                    match self {
                        #(#sample_group_arms),*
                    }
                }
            }
        };
    }
}

fn generate_write_arms(
    entry_name: &Ident,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> Vec<Ts2> {
    let tag_name = root_attrs
        .tag
        .as_ref()
        .map(|tag| tag.field_name(root_attrs));

    variants
        .iter()
        .map(|variant| {
            let variant_ident = &variant.ident;

            let tag_write = tag_name.as_ref().map(|tag_name| {
                let (extra, name) = make_inflect(
                    &make_ns(root_attrs.rename_all, variant.ident.span()),
                    variant.ident.span(),
                    |style| style.apply(tag_name),
                );
                let value = crate::inflect::inflect_no_prefix(root_attrs, variant);
                quote! {
                    #extra
                    ::metrique::writer::EntryWriter::value(writer, ::metrique::concat::const_str_value::<#name>(), #value);
                }
            });

            match &variant.data {
                Some(VariantData::Tuple(tuple_data)) => {
                    let (bindings, writes) = generate_tuple_writes(
                        tuple_data,
                        root_attrs,
                        variant.ident.span(),
                    );
                    let pattern = tuple_pattern(entry_name, variant_ident, &bindings);
                    quote::quote_spanned!(variant.ident.span()=>
                        #pattern => {
                            #tag_write
                            #(#writes)*
                        }
                    )
                }
                Some(VariantData::Struct(fields)) => {
                    let field_writes = generate_field_writes(
                        fields,
                        root_attrs,
                        |field_ident| quote! { #field_ident },
                    );
                    let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();
                    let pattern = struct_pattern(entry_name, variant_ident, &field_names, true);
                    quote::quote_spanned!(variant.ident.span()=>
                        #pattern => {
                            #tag_write
                            #(#field_writes)*
                        }
                    )
                }
                None => {
                    // Unit variant - no fields to write, just tag
                    let pattern = quote::quote_spanned!(variant.ident.span()=> #entry_name::#variant_ident);
                    quote::quote_spanned!(variant.ident.span()=>
                        #pattern => {
                            #tag_write
                        }
                    )
                }
            }
        })
        .collect()
}

fn generate_tuple_writes(
    tuple_data: &[crate::TupleData],
    root_attrs: &RootAttributes,
    variant_span: proc_macro2::Span,
) -> (Vec<Ident>, Vec<Ts2>) {
    tuple_data
        .iter()
        .enumerate()
        .map(|(idx, td)| {
            let binding = quote::format_ident!("v{}", idx);
            let write = match &td.kind {
                MetricsFieldKind::Flatten { span, prefix } => {
                    let base_ns = make_ns(root_attrs.rename_all, *span);
                    let (extra, ns) = match prefix {
                        None => (quote!(), base_ns),
                        Some(prefix) => prefix.append_to(&base_ns, variant_span),
                    };
                    quote::quote_spanned!(*span=>
                        #extra
                        ::metrique::InflectableEntry::<#ns>::write(#binding, writer);
                    )
                }
                MetricsFieldKind::FlattenEntry(span) => {
                    quote::quote_spanned!(*span=>
                        ::metrique::writer::Entry::write(#binding, writer);
                    )
                }
                MetricsFieldKind::Ignore(_) => quote!(),
                MetricsFieldKind::Timestamp(_) | MetricsFieldKind::Field { .. } => {
                    unreachable!(
                        "timestamp/plain fields are rejected earlier in tuple variant parsing"
                    )
                }
            };
            (binding, write)
        })
        .unzip()
}

fn generate_sample_group_impl(
    entry_name: &Ident,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> (Ts2, Vec<Ts2>) {
    let iter_enum_name = quote::format_ident!("{}SampleGroupIter", entry_name);
    let sample_group_arms =
        generate_sample_group_arms(entry_name, variants, root_attrs, &iter_enum_name);
    let iter_enum = generate_sample_group_iter_enum(&iter_enum_name, variants.len());
    (iter_enum, sample_group_arms)
}

fn generate_sample_group_arms(
    entry_name: &Ident,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
    iter_enum_name: &Ident,
) -> Vec<Ts2> {
    let tag_name = root_attrs
        .tag
        .as_ref()
        .map(|tag| tag.field_name(root_attrs));
    let include_tag_in_sample_group = root_attrs.tag.as_ref().is_some_and(|t| t.sample_group());

    variants.iter().enumerate().map(|(idx, variant)| {
        let variant_ident = &variant.ident;
        let iter_variant_name = quote::format_ident!("V{}", idx);

        let tag_sample_group = if let Some(tag_name) = tag_name.as_ref().filter(|_| include_tag_in_sample_group) {
            let (extra, name) = make_inflect(
                &make_ns(root_attrs.rename_all, variant.ident.span()),
                variant.ident.span(),
                |style| style.apply(tag_name),
            );
            let value = crate::inflect::inflect_no_prefix(root_attrs, variant);
            Some(quote! {
                {
                    #extra
                    ::std::iter::once((::metrique::concat::const_str_value::<#name>(), ::std::borrow::Cow::Borrowed(#value)))
                }
            })
        } else {
            None
        };

        let (pattern, mut sample_groups) = match &variant.data {
            Some(VariantData::Tuple(tuple_data)) => {
                let bindings: Vec<_> = (0..tuple_data.len()).map(|idx| quote::format_ident!("v{}", idx)).collect();
                let sample_groups: Vec<_> = tuple_data.iter().enumerate().filter_map(|(idx, td)| {
                    collect_tuple_sample_group(&td.kind, root_attrs, &bindings[idx])
                }).collect();

                (tuple_pattern(entry_name, variant_ident, &bindings), sample_groups)
            }
            Some(VariantData::Struct(fields)) => {
                let (used_fields, sample_groups): (Vec<_>, Vec<_>) = fields
                    .iter()
                    .filter_map(|field| collect_field_sample_group(field, root_attrs, |f| quote!(#f)))
                    .unzip();

                (struct_pattern(entry_name, variant_ident, &used_fields, false), sample_groups)
            }
            None => {
                // Unit variant - no fields, no sample groups
                let pattern = quote::quote_spanned!(variant.ident.span()=> #entry_name::#variant_ident);
                (pattern, vec![])
            }
        };

        if let Some(tag_sg) = tag_sample_group {
            sample_groups.insert(0, tag_sg);
        }
        let iter_expr = make_binary_tree_chain(sample_groups);

        quote::quote_spanned!(variant.ident.span()=>
            #pattern => #iter_enum_name::#iter_variant_name(#iter_expr)
        )
    }).collect()
}

fn generate_sample_group_iter_enum(iter_enum_name: &Ident, variant_count: usize) -> Ts2 {
    let iter_variants: Vec<_> = (0..variant_count)
        .map(|idx| quote::format_ident!("V{}", idx))
        .collect();

    let iter_next_arms = iter_variants
        .iter()
        .map(|variant_name| quote!(#iter_enum_name::#variant_name(iter) => iter.next()));

    quote! {
        enum #iter_enum_name<#(#iter_variants),*> {
            #(#iter_variants(#iter_variants)),*
        }

        impl<#(#iter_variants: ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)>),*> ::std::iter::Iterator for #iter_enum_name<#(#iter_variants),*> {
            type Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>);

            fn next(&mut self) -> ::std::option::Option<Self::Item> {
                match self {
                    #(#iter_next_arms),*
                }
            }
        }
    }
}

/// Helper for collecting sample groups from tuple variant fields
fn collect_tuple_sample_group(
    kind: &MetricsFieldKind,
    root_attrs: &RootAttributes,
    binding: &Ident,
) -> Option<Ts2> {
    match kind {
        MetricsFieldKind::Flatten { span, .. } => {
            let ns = make_ns(root_attrs.rename_all, *span);
            Some(quote_spanned!(*span=>
                ::metrique::InflectableEntry::<#ns>::sample_group(#binding)
            ))
        }
        MetricsFieldKind::FlattenEntry(span) => Some(quote_spanned!(*span=>
            ::metrique::writer::Entry::sample_group(#binding)
        )),
        MetricsFieldKind::Ignore(_) => None,
        MetricsFieldKind::Timestamp(_) | MetricsFieldKind::Field { .. } => {
            unreachable!("timestamp/plain fields are rejected earlier in tuple variant parsing")
        }
    }
}
