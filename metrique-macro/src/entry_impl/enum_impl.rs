use super::DescriptorFieldMeta;
use super::resolve_field_flags;
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

    let descriptor = generate_enum_descriptor(entry_name, generics, variants, root_attrs);
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
                MetricsFieldKind::Flatten { span, prefix, default_flags: flatten_default_flags } => {
                    let base_ns = make_ns(root_attrs.rename_all, *span);
                    let (extra, ns) = match prefix {
                        None => (quote!(), base_ns),
                        Some(prefix) => prefix.append_to(&base_ns, variant_span),
                    };
                    if flatten_default_flags.is_empty() {
                        quote::quote_spanned!(*span=>
                            #extra
                            ::metrique::InflectableEntry::<#ns>::write(#binding, #writer_ident);
                        )
                    } else {
                        let flag_paths: Vec<_> = flatten_default_flags.iter().map(|f| &f.path).collect();
                        let num_wrappers = flag_paths.len();
                        let wrapper_idents: Vec<_> = (0..num_wrappers)
                            .map(|i| quote::format_ident!("__metrique_fw_{}", i))
                            .collect();
                        let mut bindings = Vec::new();
                        for (i, path) in flag_paths.iter().enumerate() {
                            let ident = &wrapper_idents[i];
                            let prev = if i == 0 {
                                quote::quote! { #writer_ident }
                            } else {
                                let prev_ident = &wrapper_idents[i - 1];
                                quote::quote! { &mut #prev_ident }
                            };
                            bindings.push(quote::quote! {
                                let mut #ident = ::metrique::writer::value::ForceFlagEntryWriter::<_, #path>::new(#prev);
                            });
                        }
                        let last_wrapper = &wrapper_idents[num_wrappers - 1];
                        quote::quote_spanned!(*span=>
                            #extra
                            {
                                #(#bindings)*
                                ::metrique::InflectableEntry::<#ns>::write(#binding, &mut #last_wrapper);
                            }
                        )
                    }
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
            // Note: cfg on enum variant fields is rejected at parse time,
            // so no cfg wrapping is needed here (unlike struct fields).
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

/// Generates the per-variant `descriptors()` match arms.
///
/// Like structs, each variant's own (non-flatten) fields are split into
/// contiguous runs at flatten boundaries so segments come out in write order.
/// Run 0 additionally carries the tag field (written first) and the variant's
/// canonical name, and is always emitted; later runs are emitted only when
/// non-empty. Flattened children's segments are interleaved at their
/// declaration position.
fn generate_enum_descriptor(
    entry_name: &Ident,
    _generics: &syn::Generics,
    variants: &[MetricsVariant],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();
    let styles = NameStyle::ALL;
    let ns = make_ns(root_attrs.rename_all, entry_name.span());

    // Per-variant descriptors: each variant returns Descriptors directly.
    // No iterator enum needed since all arms return the same type.
    let match_arms: Vec<_> = variants
        .iter()
        .enumerate()
        .map(|(v_idx, variant)| {
            let variant_ident = &variant.ident;
            let variant_name = format!("{}::{}", struct_name, variant_ident);
            let mut v_timestamp_expr = quote! { None };

            // Run 0 starts with the tag field, if present: the tag value is
            // written before any variant fields.
            let mut runs: Vec<Vec<DescriptorFieldMeta>> = vec![Vec::new()];
            if let Some(tag) = &root_attrs.tag {
                // The write path inflects the tag through the propagated style.
                let tag_name = tag.field_name(root_attrs);
                let names: [String; metrique_core::Styles::COUNT] =
                    std::array::from_fn(|i| styles[i].apply(&tag_name));
                runs[0].push(DescriptorFieldMeta {
                    names,
                    flags: vec![],
                    skipped_flags: vec![],
                    explicit_unit: None,
                    field_type: syn::parse_quote!(&'static str),
                    close: true,
                    format: None,
                });
            }

            // Chain items after the base segment, in declaration order.
            // Run segments are recorded by index and materialized after the
            // walk (a timestamp field may appear after a flatten site).
            enum ChainItem {
                Run(usize),
                Child(Ts2),
            }
            let mut chain_items: Vec<ChainItem> = Vec::new();
            let close_run = |runs: &mut Vec<Vec<DescriptorFieldMeta>>,
                                 chain_items: &mut Vec<ChainItem>| {
                let run_index = runs.len() - 1;
                if run_index > 0 && !runs[run_index].is_empty() {
                    chain_items.push(ChainItem::Run(run_index));
                }
                runs.push(Vec::new());
            };

            let pattern = match &variant.data {
                Some(VariantData::Struct(fields)) => {
                    let mut flatten_bindings: Vec<&Ts2> = Vec::new();
                    for field in fields {
                        // Cfg on enum variant fields is rejected at parse time,
                        // so no cfg handling is needed here (unlike structs).
                        match &field.attrs.kind {
                            MetricsFieldKind::Ignore(_) => {}
                            MetricsFieldKind::Timestamp(_) => {
                                let ts_name = field.name.as_deref().unwrap_or("timestamp");
                                v_timestamp_expr = quote! {
                                    Some(::metrique::writer::core::TimestampDescriptor::new(#ts_name))
                                };
                            }
                            MetricsFieldKind::Field { unit, format, .. } => {
                                let names: [String; metrique_core::Styles::COUNT] =
                                    std::array::from_fn(|i| {
                                        metric_name(root_attrs, styles[i], field)
                                    });
                                let resolved = resolve_field_flags(
                                    &field.attrs.flags,
                                    &root_attrs.default_flags,
                                );
                                runs.last_mut().expect("runs is never empty").push(
                                    DescriptorFieldMeta {
                                        names,
                                        flags: resolved.flags,
                                        skipped_flags: resolved.skipped_flags,
                                        explicit_unit: unit.clone(),
                                        field_type: field.ty.clone(),
                                        close: field.attrs.close,
                                        format: format.clone(),
                                    },
                                );
                            }
                            MetricsFieldKind::Flatten { .. }
                            | MetricsFieldKind::FlattenEntry(_) => {
                                close_run(&mut runs, &mut chain_items);
                                let binding = &field.ident;
                                flatten_bindings.push(binding);
                                let child = super::flatten_descriptors_expr(
                                    &field.attrs.kind,
                                    &quote! { #binding },
                                    &ns,
                                );
                                chain_items.push(ChainItem::Child(child));
                            }
                        }
                    }
                    if flatten_bindings.is_empty() {
                        quote! { #entry_name::#variant_ident { .. } }
                    } else {
                        quote! { #entry_name::#variant_ident { #(#flatten_bindings,)* .. } }
                    }
                }
                Some(VariantData::Tuple(tds)) => {
                    // Tuple variants only contain flatten/ignore fields
                    // (plain fields are rejected at parse time).
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
                    for (i, td) in tds.iter().enumerate() {
                        if is_flatten(&td.kind) {
                            close_run(&mut runs, &mut chain_items);
                            let b = format_ident!("__v{}", i);
                            let child = super::flatten_descriptors_expr(
                                &td.kind,
                                &quote! { #b },
                                &ns,
                            );
                            chain_items.push(ChainItem::Child(child));
                        }
                    }
                    if tds.iter().any(|td| is_flatten(&td.kind)) {
                        quote! { #entry_name::#variant_ident(#(#patterns),*) }
                    } else {
                        quote! { #entry_name::#variant_ident(..) }
                    }
                }
                None => quote! { #entry_name::#variant_ident },
            };
            // Close the trailing run.
            let last_run = runs.len() - 1;
            if last_run > 0 && !runs[last_run].is_empty() {
                chain_items.push(ChainItem::Run(last_run));
            }

            // Materialize segments. Each run gets its own static block; the
            // timestamp lives on the base segment.
            let run_segment = |run: usize, ts_expr: &Ts2| {
                let ident_prefix = if run == 0 {
                    format!("V{}", v_idx)
                } else {
                    format!("V{}R{}", v_idx, run)
                };
                let desc_block = super::generate_style_matched_descriptor(
                    &runs[run],
                    &variant_name,
                    ts_expr,
                    &ident_prefix,
                );
                quote! {
                    ::metrique::writer::core::Descriptors::available(
                        ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                            #desc_block,
                            <#ns as ::metrique::NameStyle>::DESCRIPTOR_STYLE_INDEX,
                        ))
                    )
                }
            };

            let base = run_segment(0, &v_timestamp_expr);
            let none_ts = quote! { None };
            let children: Vec<(Vec<Ts2>, Ts2)> = chain_items
                .iter()
                .map(|item| match item {
                    ChainItem::Run(i) => (vec![], run_segment(*i, &none_ts)),
                    ChainItem::Child(expr) => (vec![], expr.clone()),
                })
                .collect();
            let chain_expr = super::build_descriptors_chain(base, &children);

            quote! { #pattern => #chain_expr }
        })
        .collect();

    let descriptors_method = quote! {
        fn descriptors(&self) -> ::metrique::writer::core::Descriptors<'_> {
            #[allow(deprecated)]
            match self {
                #(#match_arms),*
            }
        }
    };

    super::DescriptorOutput {
        trait_impls: quote! {},
        method: descriptors_method,
    }
}

fn is_flatten(kind: &MetricsFieldKind) -> bool {
    matches!(
        kind,
        MetricsFieldKind::Flatten { .. } | MetricsFieldKind::FlattenEntry(_)
    )
}
