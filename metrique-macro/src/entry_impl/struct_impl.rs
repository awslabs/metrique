use super::resolve_field_tags;
use super::*;
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

/// Generates descriptor infrastructure for a struct entry.
///
/// Collects field metadata, builds flatten chains with modifiers, and delegates
/// to `generate_descriptor_impl` for the `__metrique_descriptor` inherent method.
fn generate_descriptor(
    entry_name: &Ident,
    generics: &syn::Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    use super::{DescriptorFieldMeta, generate_descriptor_impl, style_const_for};
    use crate::inflect::NameStyle;

    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();
    let mut timestamp_descriptor = quote! { None };
    let mut flatten_chains = Vec::new();
    let mut flatten_tag_statics = Vec::new();
    let mut flatten_idx = 0usize;
    let mut field_metas = Vec::new();

    let styles = [
        NameStyle::Preserve,
        NameStyle::PascalCase,
        NameStyle::SnakeCase,
        NameStyle::KebabCase,
    ];

    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Ignore(_) => continue,
            MetricsFieldKind::Timestamp(_) => {
                let name = field.name.as_deref().unwrap_or("timestamp");
                timestamp_descriptor = quote! {
                    Some(::metrique::writer::core::TimestampDescriptor::__metrique_private_new(#name))
                };
            }
            MetricsFieldKind::Flatten { prefix, .. } => {
                let field_ident = &field.ident;
                // Use the concrete NS derived from the parent's rename_all (not the generic NS).
                // This matches how write() calls the child.
                let ns = make_ns(root_attrs.rename_all, field.span);

                // Merge flatten-site field_tags with parent's default_field_tags.
                // Resolution at read time: child's baked tags win, these defaults fill gaps.
                let merged_defaults =
                    resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);

                // Compute the inflected prefix at macro time.
                // Exact prefix: used as-is. Inflectable prefix: inflected to parent's style.
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
                        static #defaults_ident: [::metrique::writer::core::ResolvedFieldTag; #num_defaults] = [
                            #(#merged_defaults),*
                        ];
                    });
                    Some(quote! { .with_default_tags(&#defaults_ident) })
                } else {
                    None
                };

                if prefix_expr.is_some() || tags_expr.is_some() {
                    flatten_chains.push(quote! {
                        .chain(
                            ::metrique::InflectableEntry::<#ns>::descriptors(&self.#field_ident)
                                .map(|d| d #prefix_expr #tags_expr)
                        )
                    });
                } else {
                    flatten_chains.push(quote! {
                        .chain(::metrique::InflectableEntry::<#ns>::descriptors(&self.#field_ident))
                    });
                }
            }
            MetricsFieldKind::FlattenEntry(_) => {
                let field_ident = &field.ident;
                flatten_chains.push(quote! {
                    .chain(::metrique::writer::Entry::descriptors(&self.#field_ident))
                });
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

    // The struct uses its own rename_all style (hardcoded at macro time).
    let own_style = style_const_for(root_attrs.rename_all);

    let descriptors_method = quote! {
        fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
            #(#flatten_tag_statics)*
            ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                #entry_name::__metrique_descriptor(#own_style)
            ))
            #(#flatten_chains)*
        }
    };

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
