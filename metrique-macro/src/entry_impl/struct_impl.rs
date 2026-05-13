use super::resolve_field_tags;
use super::*;
use super::{DescriptorFieldMeta, generate_descriptor_impl, style_const_for};
use crate::inflect::NameStyle;
use crate::inflect::metric_name;

pub(crate) fn generate_struct_entry_impl(
    entry_name: &Ident,
    generics: &syn::Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let writes = generate_write_statements(fields, root_attrs);
    let sample_groups = generate_sample_group_statements(fields, root_attrs);

    // Generate descriptor infrastructure: a __metrique_descriptor(style) method with 4 statics
    // (one per name style), and a descriptors() method that selects the right one.
    let descriptor = generate_descriptor(entry_name, generics, fields, root_attrs);

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

    let write_fn = quote_spanned! {mixed=>
        fn write<'__metrique_write>(&'__metrique_write self, #writer_ident: &mut impl ::metrique::writer::EntryWriter<'__metrique_write>) {
            let #self_ident = self;
            #(#writes)*
        }
    };

    let sample_group_fn = quote_spanned! {mixed=>
        fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
            let #self_ident = self;
            #sample_groups
        }
    };

    let descriptor_trait_impls = &descriptor.trait_impls;
    let descriptors_method = &descriptor.method;

    quote! {
        const _: () = {
            // Descriptor: __metrique_descriptor(style) method with 4 statics.
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

/// Builds the flatten chain entries for the `descriptors()` method.
///
/// Each flatten field produces either:
/// - A normal chain (`.chain(child.descriptors())`) for non-cfg fields
/// - A cfg-gated let-rebinding (`#[cfg(...)] let __desc = __desc.chain(...)`) for cfg fields
///
/// Returns `(flatten_tag_statics, flatten_chains)` where each chain is `(is_cfg_gated, tokens)`.
/// Builds flatten chain entries for the descriptors() method.
///
/// Unlike sample_group (which uses make_binary_tree_chain to avoid deep type nesting
/// when many fields are chained), descriptor flatten chains are linear. This is fine
/// because flatten depth is bounded by the number of flatten fields in one struct,
/// which is typically 1-3 in practice.
fn build_flatten_chains(
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> (Vec<Ts2>, Vec<(bool, Ts2)>) {
    let mut flatten_tag_statics = Vec::new();
    let mut flatten_chains: Vec<(bool, Ts2)> = Vec::new();
    let mut flatten_idx = 0usize;

    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Flatten { prefix, .. } => {
                let field_ident = &field.ident;
                let cfg_attrs: Vec<_> = field.cfg_attrs().collect();
                let ns = make_ns(root_attrs.rename_all, field.span);

                let merged_defaults =
                    resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);

                let prefix_expr = prefix.as_ref().map(|pfx| {
                    let inflected = pfx.apply_prefix_only(root_attrs.rename_all);
                    quote! { .with_prefix(#inflected) }
                });

                let tags_expr = if !merged_defaults.is_empty() {
                    let num_defaults = merged_defaults.len();
                    let defaults_ident =
                        format_ident!("__METRIQUE_FLATTEN_DEFAULTS_{}", flatten_idx);
                    flatten_idx += 1;
                    flatten_tag_statics.push(quote! {
                        static #defaults_ident: [::metrique::writer::core::FieldTag; #num_defaults] = [
                            #(#merged_defaults),*
                        ];
                    });
                    Some(quote! { .with_default_tags(&#defaults_ident) })
                } else {
                    None
                };

                let child_expr = if prefix_expr.is_some() || tags_expr.is_some() {
                    quote! {
                        ::metrique::InflectableEntry::<#ns>::descriptors(&self.#field_ident)
                            .map(|d| d #prefix_expr #tags_expr)
                    }
                } else {
                    quote! {
                        ::metrique::InflectableEntry::<#ns>::descriptors(&self.#field_ident)
                    }
                };

                if cfg_attrs.is_empty() {
                    flatten_chains.push((false, quote! { .chain(#child_expr) }));
                } else {
                    flatten_chains.push((
                        true,
                        quote! {
                            #(#cfg_attrs)* let __desc = __desc.chain(#child_expr);
                        },
                    ));
                }
            }
            MetricsFieldKind::FlattenEntry(_) => {
                let field_ident = &field.ident;
                let cfg_attrs: Vec<_> = field.cfg_attrs().collect();
                let child_expr = quote! {
                    ::metrique::writer::Entry::descriptors(&self.#field_ident)
                };
                if cfg_attrs.is_empty() {
                    flatten_chains.push((false, quote! { .chain(#child_expr) }));
                } else {
                    flatten_chains.push((
                        true,
                        quote! {
                            #(#cfg_attrs)* let __desc = __desc.chain(#child_expr);
                        },
                    ));
                }
            }
            _ => {}
        }
    }

    (flatten_tag_statics, flatten_chains)
}

/// Assembles the `descriptors()` method body from the entry's own descriptor
/// and any flatten chains.
///
/// When all chains are non-cfg, generates a simple expression chain.
/// When cfg-gated chains exist, uses let-rebinding so cfg-disabled fields
/// are excluded without affecting the iterator type.
fn assemble_descriptors_method(
    entry_name: &Ident,
    own_style: &Ts2,
    flatten_tag_statics: &[Ts2],
    flatten_chains: &[(bool, Ts2)],
) -> Ts2 {
    let has_cfg = flatten_chains.iter().any(|(cfg, _)| *cfg);

    if has_cfg {
        let normal: Vec<_> = flatten_chains
            .iter()
            .filter_map(|(cfg, t)| if !cfg { Some(t) } else { None })
            .collect();
        let cfg_gated: Vec<_> = flatten_chains
            .iter()
            .filter_map(|(cfg, t)| if *cfg { Some(t) } else { None })
            .collect();
        quote! {
            fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
                #(#flatten_tag_statics)*
                let __desc = ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                    #entry_name::__metrique_descriptor(#own_style)
                ))
                #(#normal)*;
                #(#cfg_gated)*
                __desc
            }
        }
    } else {
        let chains: Vec<_> = flatten_chains.iter().map(|(_, t)| t).collect();
        quote! {
            fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
                #(#flatten_tag_statics)*
                ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                    #entry_name::__metrique_descriptor(#own_style)
                ))
                #(#chains)*
            }
        }
    }
}

/// Generates descriptor infrastructure for a struct entry.
///
/// Collects field metadata (names in 4 styles, tags, units), builds flatten chains
/// with modifiers, and delegates to shared helpers for the `__metrique_descriptor`
/// method and the `descriptors()` method body.
fn generate_descriptor(
    entry_name: &Ident,
    generics: &syn::Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();
    let mut timestamp_descriptor = quote! { None };
    let mut field_metas = Vec::new();
    let styles = NameStyle::DESCRIPTOR_STYLES;

    // Collect field metadata and timestamp
    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Ignore(_)
            | MetricsFieldKind::Flatten { .. }
            | MetricsFieldKind::FlattenEntry(_) => continue,
            MetricsFieldKind::Timestamp(_) => {
                let name = field.name.as_deref().unwrap_or("timestamp");
                timestamp_descriptor = quote! {
                    Some(::metrique::writer::core::TimestampDescriptor::__metrique_private_new(#name))
                };
            }
            MetricsFieldKind::Field { unit, .. } => {
                let names: [String; 4] =
                    std::array::from_fn(|i| metric_name(root_attrs, styles[i], field));
                let tags =
                    resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);
                let unit_expr = match unit {
                    Some(u) => {
                        quote! { Some(<#u as ::metrique::writer::core::unit::UnitTag>::UNIT) }
                    }
                    None => quote! { None },
                };
                field_metas.push(DescriptorFieldMeta {
                    names,
                    tags,
                    unit_expr,
                });
            }
        }
    }

    let descriptor_impl = generate_descriptor_impl(
        entry_name,
        generics,
        &struct_name,
        &field_metas,
        &timestamp_descriptor,
    );

    let own_style = style_const_for(root_attrs.rename_all);
    let (flatten_tag_statics, flatten_chains) = build_flatten_chains(fields, root_attrs);
    let descriptors_method = assemble_descriptors_method(
        entry_name,
        &own_style,
        &flatten_tag_statics,
        &flatten_chains,
    );

    super::DescriptorOutput {
        trait_impls: descriptor_impl,
        method: descriptors_method,
    }
}

fn generate_write_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Vec<Ts2> {
    let mut writes = Vec::new();
    let writer_ident = mixed_site_writer();
    let self_ident = mixed_site_self();

    for field_ident in root_attrs.configuration_field_names() {
        writes.push(quote! {
            ::metrique::writer::Entry::write(&#self_ident.#field_ident, #writer_ident);
        });
    }

    writes.extend(generate_field_writes(
        fields,
        root_attrs,
        |field_ident| quote! { &#self_ident.#field_ident },
    ));
    writes
}

fn generate_sample_group_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Ts2 {
    let self_ident = mixed_site_self();

    let sample_group_fields: Vec<_> = fields
        .iter()
        .filter_map(|field| {
            collect_field_sample_group(field, root_attrs, |f| quote! { &#self_ident.#f })
                .map(|(_, iter)| iter)
        })
        .collect();

    make_binary_tree_chain(sample_group_fields)
}
