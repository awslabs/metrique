use super::resolve_field_flags;
use super::*;
use super::{DescriptorFieldMeta, generate_descriptor_impl};
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
    let styles = NameStyle::ALL;

    // Collect field metadata and timestamp
    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Ignore(_)
            | MetricsFieldKind::Flatten { .. }
            | MetricsFieldKind::FlattenEntry(_) => continue,
            MetricsFieldKind::Timestamp(_) => {
                let name = field.name.as_deref().unwrap_or("timestamp");
                timestamp_descriptor = quote! {
                    Some(::metrique::writer::core::TimestampDescriptor::new(#name))
                };
            }
            MetricsFieldKind::Field { unit, .. } => {
                let names: [String; 4] =
                    std::array::from_fn(|i| metric_name(root_attrs, styles[i], field));
                let resolved = resolve_field_flags(&field.attrs.flags, &root_attrs.default_flags);
                let unit_expr = match unit {
                    Some(u) => {
                        quote! { Some(<#u as ::metrique::writer::core::unit::UnitTag>::UNIT) }
                    }
                    None => quote! { None },
                };
                field_metas.push(DescriptorFieldMeta {
                    names,
                    flags: resolved.flags,
                    skipped_flags: resolved.skipped_flags,
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

    let own_style_ns = make_ns(root_attrs.rename_all, entry_name.span());
    let flatten_chains = build_flatten_chains(fields, root_attrs);
    let descriptors_method =
        assemble_descriptors_method(entry_name, &own_style_ns, &flatten_chains);

    super::DescriptorOutput {
        trait_impls: descriptor_impl,
        method: descriptors_method,
    }
}

/// Builds the flatten chain entries for the `descriptors()` method.
///
/// Each flatten field produces either:
/// - A normal chain (`.chain(child.descriptors())`) for non-cfg fields
/// - A cfg-gated let-rebinding (`#[cfg(...)] let __desc = __desc.chain(...)`) for cfg fields
///
/// Builds flatten chain entries for the descriptors() method.
///
/// Returns flatten_chains where each non-cfg chain is a full
/// iterator expression (used with make_binary_tree_chain for balanced type nesting).
/// Cfg-gated chains use let-rebinding and are applied after the tree.
fn build_flatten_chains(
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Vec<(Vec<Ts2>, Ts2)> {
    let mut flatten_chains: Vec<(Vec<Ts2>, Ts2)> = Vec::new();

    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Flatten {
                prefix,
                default_flags: flatten_default_flags,
                ..
            } => {
                let field_ident = &field.ident;
                let cfg_attrs: Vec<_> = field.cfg_attrs().collect();
                let ns = make_ns(root_attrs.rename_all, field.span);

                let prefix_expr = prefix.as_ref().map(|pfx| {
                    // Generate a per-style prefix array so the correct inflection
                    // is selected at runtime based on the parent's propagated style.
                    let inflected: Vec<String> = crate::inflect::NameStyle::ALL
                        .iter()
                        .map(|s| pfx.apply_prefix_only(*s))
                        .collect();
                    quote! {
                        .with_prefix(
                            [#(#inflected),*][<#ns as ::metrique::NameStyle>::DESCRIPTOR_STYLE_INDEX as usize]
                        )
                    }
                });

                let extra_flags_expr = if flatten_default_flags.is_empty() {
                    None
                } else {
                    let flag_exprs: Vec<_> = flatten_default_flags
                        .iter()
                        .map(|f| {
                            let path = &f.path;
                            quote! { ::metrique::writer::core::FieldFlag::new::<#path>() }
                        })
                        .collect();
                    let num_flags = flag_exprs.len();
                    Some(quote! {
                        .with_extra_flags({
                            static __FLATTEN_FLAGS: [::metrique::writer::core::FieldFlag; #num_flags] = [
                                #(#flag_exprs),*
                            ];
                            &__FLATTEN_FLAGS
                        })
                    })
                };

                let has_transforms = prefix_expr.is_some() || extra_flags_expr.is_some();
                let child_expr = if has_transforms {
                    quote! {
                        ::metrique::InflectableEntry::<#ns>::descriptors(&self.#field_ident)
                            .map_available(|d| d #prefix_expr #extra_flags_expr)
                    }
                } else {
                    quote! {
                        ::metrique::InflectableEntry::<#ns>::descriptors(&self.#field_ident)
                    }
                };

                flatten_chains.push((
                    cfg_attrs.iter().map(|a| quote! { #a }).collect(),
                    child_expr,
                ));
            }
            MetricsFieldKind::FlattenEntry(_) => {
                let field_ident = &field.ident;
                let cfg_attrs: Vec<_> = field.cfg_attrs().collect();
                let child_expr = quote! {
                    ::metrique::writer::Entry::descriptors(&self.#field_ident)
                };
                flatten_chains.push((
                    cfg_attrs.iter().map(|a| quote! { #a }).collect(),
                    child_expr,
                ));
            }
            _ => {}
        }
    }

    flatten_chains
}

/// Assembles the `descriptors()` method body from the entry's own descriptor
/// and any flatten chains.
///
/// When all chains are non-cfg, generates a simple expression chain.
/// When cfg-gated chains exist, uses let-rebinding so cfg-disabled fields
/// are excluded without affecting the iterator type.
fn assemble_descriptors_method(
    entry_name: &Ident,
    own_style_ns: &Ts2,
    flatten_chains: &[(Vec<Ts2>, Ts2)],
) -> Ts2 {
    let base_expr = quote! {
        ::metrique::writer::core::Descriptors::available(
            ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                #entry_name::__metrique_descriptor(),
                <#own_style_ns as ::metrique::NameStyle>::DESCRIPTOR_STYLE_INDEX,
            ))
        )
    };

    let chain_expr = super::build_descriptors_chain(base_expr, flatten_chains);

    quote! {
        fn descriptors(&self) -> ::metrique::writer::core::Descriptors<'_> {
            #chain_expr
        }
    }
}
