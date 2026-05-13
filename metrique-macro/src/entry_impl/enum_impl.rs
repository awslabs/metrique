use super::DescriptorFieldMeta;
use super::resolve_field_tags;
use super::*;
use crate::enums::{MetricsVariant, VariantData};
use crate::inflect::NameStyle;
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

/// Generates the enum iterator type for per-variant descriptor dispatch.
/// Same pattern as sample_group: one variant per enum arm, unified via Iterator impl.
fn generate_enum_descriptor(
    entry_name: &Ident,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();
    let styles = NameStyle::DESCRIPTOR_STYLES;
    let ns = make_ns(root_attrs.rename_all, entry_name.span());

    // Per-variant descriptors: each variant yields its own descriptor containing only
    // that variant's fields (+ tag field), plus any flatten children's descriptors.
    let iter_enum_name = format_ident!("{}DescIter", entry_name);
    let (iter_variants, iter_enum_def) =
        generate_descriptor_iter_enum(&iter_enum_name, variants.len());

    // Generate per-variant match arms, each with its own static descriptor.
    let match_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(v_idx, variant)| {
            let iter_v = &iter_variants[v_idx];
            let variant_ident = &variant.ident;

            // Collect this variant's non-flatten fields
            let mut v_field_metas = Vec::new();
            if let Some(tag) = &root_attrs.tag {
                let names: [String; 4] = std::array::from_fn(|_| tag.field_name(root_attrs));
                v_field_metas.push(DescriptorFieldMeta { names, tags: vec![], unit_expr: quote! { None } });
            }
            if let Some(VariantData::Struct(fields)) = &variant.data {
                for field in fields {
                    if let MetricsFieldKind::Field { unit, .. } = &field.attrs.kind {
                            let names: [String; 4] = std::array::from_fn(|i| metric_name(root_attrs, styles[i], field));
                            let tags = resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);
                            let unit_expr = match unit {
                                Some(u) => quote! { Some(<#u as ::metrique::writer::core::unit::UnitTag>::UNIT) },
                                None => quote! { None },
                            };
                            v_field_metas.push(DescriptorFieldMeta { names, tags, unit_expr });
                    }
                }
            }

            // Generate this variant's static descriptor (inside the match arm)
            let v_desc_ident = format_ident!("__METRIQUE_VDESC_{}", v_idx);
            let v_fields_ident = format_ident!("__METRIQUE_VFIELDS_{}", v_idx);
            let variant_name = format!("{}::{}", struct_name, variant_ident);
            let num_v_fields = v_field_metas.len();

            let v_tag_statics: Vec<_> = v_field_metas.iter().enumerate().map(|(i, f)| {
                let ident = format_ident!("__METRIQUE_VTAGS_{}_{}", v_idx, i);
                let tags = &f.tags;
                let num_tags = tags.len();
                quote! { static #ident: [::metrique::writer::core::FieldTag; #num_tags] = [#(#tags),*]; }
            }).collect();

            let style_idx = root_attrs.rename_all.descriptor_index();
            let v_field_exprs: Vec<_> = v_field_metas.iter().enumerate().map(|(i, f)| {
                let name = &f.names[style_idx];
                let tags_ident = format_ident!("__METRIQUE_VTAGS_{}_{}", v_idx, i);
                let unit_expr = &f.unit_expr;
                quote! {
                    ::metrique::writer::core::FieldDescriptor::__metrique_private_new(
                        #name, &#tags_ident, ::metrique::writer::core::FieldShape::Opaque, #unit_expr,
                    )
                }
            }).collect();

            let base = quote! {
                {
                    #(#v_tag_statics)*
                    static #v_fields_ident: [::metrique::writer::core::FieldDescriptor; #num_v_fields] = [#(#v_field_exprs),*];
                    static #v_desc_ident: ::metrique::writer::core::EntryDescriptor =
                        ::metrique::writer::core::EntryDescriptor::__metrique_private_new(#variant_name, &#v_fields_ident, None);
                    ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(&#v_desc_ident))
                }
            };

            let (pattern, chain_expr) = build_variant_descriptor_arm(entry_name, variant, &base, &ns);
            quote! { #pattern => #iter_enum_name::#iter_v(#chain_expr) }
        })
        .collect();

    let descriptors_method = quote! {
        fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
            #[allow(deprecated)]
            match self {
                #(#match_arms),*
            }
        }
    };

    super::DescriptorOutput {
        trait_impls: quote! { #iter_enum_def },
        method: descriptors_method,
    }
}

fn generate_descriptor_iter_enum(
    iter_enum_name: &Ident,
    variant_count: usize,
) -> (Vec<Ident>, Ts2) {
    let iter_variants: Vec<_> = (0..variant_count)
        .map(|i| format_ident!("V{}", i))
        .collect();

    let iter_next_arms: Vec<_> = iter_variants
        .iter()
        .map(|v| quote! { #iter_enum_name::#v(iter) => iter.next() })
        .collect();

    let iter_enum_def = quote! {
        enum #iter_enum_name<'__metrique_lt, #(#iter_variants),*> {
            #(#iter_variants(#iter_variants),)*
            __Phantom(::std::marker::PhantomData<&'__metrique_lt ()>),
        }
        impl<'__metrique_lt, #(#iter_variants: ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'__metrique_lt>>),*>
            ::std::iter::Iterator for #iter_enum_name<'__metrique_lt, #(#iter_variants),*>
        {
            type Item = ::metrique::writer::core::DescriptorRef<'__metrique_lt>;
            fn next(&mut self) -> ::std::option::Option<Self::Item> {
                match self {
                    #(#iter_next_arms,)*
                    #iter_enum_name::__Phantom(_) => None,
                }
            }
        }
    };

    (iter_variants, iter_enum_def)
}

fn is_flatten(kind: &MetricsFieldKind) -> bool {
    matches!(
        kind,
        MetricsFieldKind::Flatten { .. } | MetricsFieldKind::FlattenEntry(_)
    )
}

/// Generates a `.chain(child.descriptors())` expression for a flatten field.
fn flatten_chain_expr(field_kind: &MetricsFieldKind, binding: &Ts2, ns: &Ts2) -> Ts2 {
    match field_kind {
        MetricsFieldKind::Flatten { .. } => {
            quote! { .chain(::metrique::InflectableEntry::<#ns>::descriptors(#binding)) }
        }
        MetricsFieldKind::FlattenEntry(_) => {
            quote! { .chain(::metrique::writer::Entry::descriptors(#binding)) }
        }
        _ => unreachable!(),
    }
}

/// Generates a match arm pattern and iterator expression for one enum variant's descriptors.
///
/// Generates a match arm pattern and iterator expression for one enum variant.
///
/// Takes the base iterator and appends flatten children's descriptors if the variant has them.
/// Returns (pattern, chain_expr) for use in the generated match.
fn build_variant_descriptor_arm(
    entry_name: &Ident,
    variant: &MetricsVariant,
    base: &Ts2,
    ns: &Ts2,
) -> (Ts2, Ts2) {
    let variant_ident = &variant.ident;

    match &variant.data {
        Some(VariantData::Struct(fields)) => {
            let flatten_fields: Vec<_> = fields
                .iter()
                .filter(|f| is_flatten(&f.attrs.kind))
                .collect();

            if flatten_fields.is_empty() {
                (quote! { #entry_name::#variant_ident { .. } }, base.clone())
            } else {
                let bindings: Vec<_> = flatten_fields.iter().map(|f| &f.ident).collect();
                let chains: Vec<_> = flatten_fields
                    .iter()
                    .map(|f| flatten_chain_expr(&f.attrs.kind, &f.ident, ns))
                    .collect();
                (
                    quote! { #entry_name::#variant_ident { #(#bindings),*, .. } },
                    quote! { #base #(#chains)* },
                )
            }
        }
        Some(VariantData::Tuple(tds)) => {
            if !tds.iter().any(|td| is_flatten(&td.kind)) {
                return (quote! { #entry_name::#variant_ident(..) }, base.clone());
            }

            let patterns: Vec<_> = tds
                .iter()
                .enumerate()
                .map(|(i, td)| {
                    if is_flatten(&td.kind) {
                        let b = format_ident!("__v{}", i);
                        quote! { #b }
                    } else {
                        quote! { _ }
                    }
                })
                .collect();

            let chains: Vec<_> = tds
                .iter()
                .enumerate()
                .filter(|(_, td)| is_flatten(&td.kind))
                .map(|(i, td)| {
                    let b = format_ident!("__v{}", i);
                    flatten_chain_expr(&td.kind, &quote! { #b }, ns)
                })
                .collect();

            (
                quote! { #entry_name::#variant_ident(#(#patterns),*) },
                quote! { #base #(#chains)* },
            )
        }
        None => (quote! { #entry_name::#variant_ident }, base.clone()),
    }
}
