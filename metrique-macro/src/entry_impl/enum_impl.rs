use super::resolve_field_tags;
use super::*;
use crate::enums::{MetricsVariant, VariantData};
use crate::inflect::metric_name;

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
    generics: &syn::Generics,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let write_arms = generate_write_arms(entry_name, variants, root_attrs);
    let (iter_enum, sample_group_arms) =
        generate_sample_group_impl(entry_name, variants, root_attrs);

    // Add NS as an additional generic parameter
    let mut impl_generics = generics.clone();
    impl_generics
        .params
        .push(syn::parse_quote!(NS: ::metrique::NameStyle));
    let (impl_generics, _, _) = impl_generics.split_for_impl();
    let (_, ty_generics, where_clause) = generics.split_for_impl();

    let mixed = proc_macro2::Span::mixed_site();
    let writer_ident = mixed_site_writer();
    let self_ident = mixed_site_self();

    // Macro hygiene pattern: see `mixed_site_writer` / `mixed_site_self` docs in `entry_impl.rs`.
    let write_fn = quote_spanned! {mixed=>
        fn write<'__metrique_write>(&'__metrique_write self, #writer_ident: &mut impl ::metrique::writer::EntryWriter<'__metrique_write>) {
            let #self_ident = self;
            #[allow(deprecated)]
            match #self_ident {
                #(#write_arms)*
            }
        }
    };

    let sample_group_fn = quote_spanned! {mixed=>
        fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
            let #self_ident = self;
            match #self_ident {
                #(#sample_group_arms),*
            }
        }
    };

    let descriptor = generate_enum_descriptor(entry_name, variants, root_attrs);
    let descriptor_trait_impls = &descriptor.trait_impls;
    let descriptors_method = &descriptor.method;

    quote! {
        const _: () = {
            #iter_enum

            #descriptor_trait_impls

            #[expect(deprecated)]
            impl #impl_generics ::metrique::InflectableEntry<NS> for #entry_name #ty_generics #where_clause {
                #write_fn
                #sample_group_fn
                #descriptors_method
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
    let writer_ident = mixed_site_writer();

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
                    ::metrique::writer::EntryWriter::value(#writer_ident, ::metrique::concat::const_str_value::<#name>(), #value);
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
    let writer_ident = mixed_site_writer();
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
                        ::metrique::InflectableEntry::<#ns>::write(#binding, #writer_ident);
                    )
                }
                MetricsFieldKind::FlattenEntry(span) => {
                    quote::quote_spanned!(*span=>
                        ::metrique::writer::Entry::write(#binding, #writer_ident);
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

fn generate_enum_descriptor(
    entry_name: &Ident,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    // Returns (trait_impls, descriptors_method) same as struct version.
    use crate::inflect::NameStyle;
    use std::collections::BTreeSet;

    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();

    struct FieldInfo {
        names: [String; 4],
        tags: Vec<Ts2>,
        tags_ident: Ident,
        unit_expr: Ts2,
    }
    let mut field_infos = Vec::new();
    let mut field_idx = 0usize;

    let styles = [
        NameStyle::Preserve,
        NameStyle::PascalCase,
        NameStyle::SnakeCase,
        NameStyle::KebabCase,
    ];

    // Tag field comes first (if present)
    if let Some(tag) = &root_attrs.tag {
        let names: [String; 4] = std::array::from_fn(|_| tag.field_name(root_attrs));
        let tags_ident = format_ident!("__METRIQUE_TAGS_{}", field_idx);
        field_infos.push(FieldInfo {
            names,
            tags: vec![],
            tags_ident,
            unit_expr: quote! { None },
        });
        field_idx += 1;
    }

    // Union of all variant fields (deduplicated by preserve-style name)
    let mut seen_names = BTreeSet::new();
    for variant in variants {
        let fields = match &variant.data {
            Some(VariantData::Struct(fields)) => fields.as_slice(),
            _ => continue,
        };
        for field in fields {
            match &field.attrs.kind {
                MetricsFieldKind::Ignore(_) | MetricsFieldKind::Timestamp(_) => continue,
                MetricsFieldKind::Flatten { .. } | MetricsFieldKind::FlattenEntry(_) => continue,
                MetricsFieldKind::Field { unit, .. } => {
                    let preserve_name = metric_name(root_attrs, NameStyle::Preserve, field);
                    if !seen_names.insert(preserve_name) {
                        continue;
                    }

                    let names: [String; 4] =
                        std::array::from_fn(|i| metric_name(root_attrs, styles[i], field));

                    let tags =
                        resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);
                    let tags_ident = format_ident!("__METRIQUE_TAGS_{}", field_idx);

                    let unit_expr = match unit {
                        Some(u) => {
                            quote! { Some(<#u as ::metrique::writer::core::unit::UnitTag>::UNIT) }
                        }
                        None => quote! { None },
                    };

                    field_infos.push(FieldInfo {
                        names,
                        tags,
                        tags_ident,
                        unit_expr,
                    });
                    field_idx += 1;
                }
            }
        }
    }

    let num_fields = field_infos.len();

    // Tag statics (shared across all 4 descriptors)
    let tag_statics: Vec<Ts2> = field_infos
        .iter()
        .map(|fi| {
            let ident = &fi.tags_ident;
            let tags = &fi.tags;
            let num_tags = tags.len();
            quote! {
                static #ident: [::metrique::writer::core::ResolvedFieldTag; #num_tags] = [
                    #(#tags),*
                ];
            }
        })
        .collect();

    // Generate 4 trait impls
    let style_names = ["PRESERVE", "PASCAL", "SNAKE", "KEBAB"];

    let trait_impls: Vec<Ts2> = (0..4).map(|style_idx| {
        let desc_ident = format_ident!("__METRIQUE_DESC_{}", style_names[style_idx]);
        let fields_ident = format_ident!("__METRIQUE_FIELDS_{}", style_names[style_idx]);
        let style_idx_u8 = style_idx as u8;

        let field_exprs: Vec<Ts2> = field_infos.iter().map(|fi| {
            let name = &fi.names[style_idx];
            let tags_ident = &fi.tags_ident;
            let unit_expr = &fi.unit_expr;
            quote! {
                ::metrique::writer::core::FieldDescriptor::__metrique_private_new(
                    #name,
                    &#tags_ident,
                    ::metrique::writer::core::FieldShape::Opaque,
                    #unit_expr,
                )
            }
        }).collect();

        quote! {
            #style_idx_u8 => {
                #(#tag_statics)*
                static #fields_ident: [::metrique::writer::core::FieldDescriptor; #num_fields] = [
                    #(#field_exprs),*
                ];
                static #desc_ident: ::metrique::writer::core::EntryDescriptor =
                    ::metrique::writer::core::EntryDescriptor::__metrique_private_new(
                        #struct_name,
                        &#fields_ident,
                        None,
                    );
                &#desc_ident
            }
        }
    }).collect();

    let descriptor_impl = quote! {
        impl #entry_name {
            #[doc(hidden)]
            fn __metrique_descriptor(__style: u8) -> &'static ::metrique::writer::core::EntryDescriptor {
                match __style {
                    #(#trait_impls)*
                    _ => unreachable!()
                }
            }
        }
    };

    let descriptors_method = quote! {
        fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
            ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                #entry_name::__metrique_descriptor(NS::__STYLE_INDEX)
            ))
        }
    };

    super::DescriptorOutput {
        trait_impls: descriptor_impl,
        method: descriptors_method,
    }
}
