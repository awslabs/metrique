use super::*;
use crate::enums::{MetricsVariant, VariantData};

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
    let tag_field = root_attrs
        .tag
        .as_ref()
        .map(|tag| format_ident!("{}", tag.name));

    variants
        .iter()
        .map(|variant| {
            let variant_ident = &variant.ident;
            let tag_write = tag_field.as_ref().map(|tag| {
                let (extra, name) = make_inflect(
                    &make_ns(root_attrs.rename_all, variant.ident.span()),
                    format_ident!("Inflect", span = variant.ident.span()),
                    &tag.to_string(),
                    variant.ident.span(),
                    |style| style.apply(&tag.to_string()),
                );
                quote! {
                    #extra
                    ::metrique::writer::EntryWriter::value(writer, ::metrique::concat::const_str_value::<#name>(), #tag);
                }
            });

            match &variant.data {
                Some(VariantData::Tuple(tuple_data)) => {
                    let (bindings, writes) = generate_tuple_writes(
                        tuple_data,
                        variant_ident,
                        root_attrs,
                        variant.ident.span(),
                    );
                    let all_bindings = if let Some(tag) = tag_field.as_ref() {
                        quote!(#tag, #(#bindings),*)
                    } else {
                        quote!(#(#bindings),*)
                    };
                    quote::quote_spanned!(variant.ident.span()=>
                        #entry_name::#variant_ident(#all_bindings) => {
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
                    let pattern = if let Some(tag) = tag_field.as_ref() {
                        quote!(#entry_name::#variant_ident { #tag, #(#field_names),* })
                    } else {
                        quote!(#entry_name::#variant_ident { #(#field_names),* })
                    };
                    quote::quote_spanned!(variant.ident.span()=>
                        #pattern => {
                            #tag_write
                            #(#field_writes)*
                        }
                    )
                }
                None => unreachable!("unit variants are rejected by entry enum parsing"),
            }
        })
        .collect()
}

fn generate_tuple_writes(
    tuple_data: &[crate::TupleData],
    variant_ident: &Ident,
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
                    let base_name = format!("{}_{}", variant_ident, idx);
                    let (extra, ns) = match prefix {
                        None => (quote!(), base_ns),
                        Some(crate::Prefix::Inflectable { prefix }) => {
                            make_inflect_prefix(&base_ns, prefix, &base_name, variant_span)
                        }
                        Some(crate::Prefix::Exact(exact_prefix)) => {
                            make_exact_prefix(&base_ns, exact_prefix, &base_name, variant_span)
                        }
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
    let tag_field = root_attrs
        .tag
        .as_ref()
        .map(|tag| format_ident!("{}", tag.name));
    let include_tag_in_sample_group = root_attrs
        .tag
        .as_ref()
        .is_some_and(|t| t.sample_group.is_present());

    variants.iter().enumerate().map(|(idx, variant)| {
        let variant_ident = &variant.ident;
        let iter_variant_name = quote::format_ident!("V{}", idx);

        let tag_sample_group = if let Some(tag) = tag_field.as_ref().filter(|_| include_tag_in_sample_group) {
            let (extra, name) = make_inflect(
                &make_ns(root_attrs.rename_all, variant.ident.span()),
                format_ident!("Inflect", span = variant.ident.span()),
                &tag.to_string(),
                variant.ident.span(),
                |style| style.apply(&tag.to_string()),
            );
            Some(quote! {
                {
                    #extra
                    ::std::iter::once((::metrique::concat::const_str_value::<#name>(), #tag.clone()))
                }
            })
        } else {
            None
        };

        match &variant.data {
            Some(VariantData::Tuple(tuple_data)) => {
                let bindings: Vec<_> = (0..tuple_data.len()).map(|idx| quote::format_ident!("v{}", idx)).collect();
                let mut sample_groups: Vec<_> = tuple_data.iter().enumerate().filter_map(|(idx, td)| {
                    collect_tuple_sample_group(&td.kind, root_attrs, &bindings[idx])
                }).collect();

                if let Some(tag_sg) = tag_sample_group {
                    sample_groups.insert(0, tag_sg);
                }

                let all_bindings = if let Some(tag) = tag_field.as_ref() {
                    quote!(#tag, #(#bindings),*)
                } else {
                    quote!(#(#bindings),*)
                };

                let iter_expr = if sample_groups.is_empty() {
                    quote!(::std::iter::empty())
                } else {
                    make_binary_tree_chain(sample_groups)
                };

                quote::quote_spanned!(variant.ident.span()=>
                    #entry_name::#variant_ident(#all_bindings) => #iter_enum_name::#iter_variant_name(#iter_expr)
                )
            }
            Some(VariantData::Struct(fields)) => {
                let (used_fields, mut sample_groups): (Vec<_>, Vec<_>) = fields
                    .iter()
                    .filter_map(|field| collect_field_sample_group(field, root_attrs, |f| quote!(#f)))
                    .unzip();

                if let Some(tag_sg) = tag_sample_group {
                    sample_groups.insert(0, tag_sg);
                }

                let pattern = match (tag_field.as_ref(), used_fields.is_empty()) {
                    (Some(tag), true) => quote!(#entry_name::#variant_ident { #tag, .. }),
                    (Some(tag), false) => {
                        quote!(#entry_name::#variant_ident { #tag, #(#used_fields),*, .. })
                    }
                    (None, true) => quote!(#entry_name::#variant_ident { .. }),
                    (None, false) => quote!(#entry_name::#variant_ident { #(#used_fields),*, .. }),
                };

                let iter_expr = if sample_groups.is_empty() {
                    quote!(::std::iter::empty())
                } else {
                    make_binary_tree_chain(sample_groups)
                };

                quote::quote_spanned!(variant.ident.span()=>
                    #pattern => #iter_enum_name::#iter_variant_name(#iter_expr)
                )
            }
            None => unreachable!("entry impls are only generated for enums with data")
        }
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
