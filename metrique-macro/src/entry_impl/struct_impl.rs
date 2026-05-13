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

    // Generate the descriptor infrastructure:
    // - 4 trait impls of __StaticStyledDescriptor (one per name style: Identity, PascalCase, SnakeCase, KebabCase)
    //   Each impl contains a static EntryDescriptor with field names resolved for that style.
    //   This enables compile-time dispatch: when a parent calls descriptors() with a specific NS,
    //   the trait resolves to the correct static without runtime branching.
    // - The descriptors() method that uses the trait + chains flattened children with modifiers.
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
            // Descriptor trait impls: one per name style, each providing a static
            // EntryDescriptor with field names resolved for that style.
            // The descriptors() method below uses compile-time trait dispatch
            // (<NS as __StaticStyledDescriptor<Self>>::descriptor()) to select
            // the right static based on the NS type parameter.
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
/// Returns a [`DescriptorOutput`] containing:
/// - `trait_impls`: 4 `__StaticStyledDescriptor` impls (one per name style), each containing
///   a static `EntryDescriptor` with field names resolved for that style.
/// - `method`: the `fn descriptors()` method body that uses compile-time trait dispatch
///   to select the right static, then chains flattened children's descriptors with
///   prefix/tag modifiers applied.
fn generate_descriptor(
    entry_name: &Ident,
    generics: &syn::Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    use crate::inflect::NameStyle;

    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();

    // Collect field metadata and flatten chains
    let mut timestamp_descriptor = quote! { None };
    let mut flatten_chains = Vec::new();
    let mut flatten_tag_statics = Vec::new();
    let mut flatten_idx = 0usize;

    struct FieldInfo {
        tags: Vec<Ts2>,
        tags_ident: Ident,
        unit_expr: Ts2,
    }
    let mut field_infos = Vec::new();
    // Collect names per style: field_names[style_idx][field_idx]
    let mut field_names: [Vec<String>; 4] = [vec![], vec![], vec![], vec![]];

    let mut field_idx = 0usize;

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
                // This matches how write() calls the child: with a specific name style,
                // not the generic parameter. The child's __StaticStyledDescriptor impl
                // for this concrete NS returns the static with correctly-styled field names.
                let ns = make_ns(root_attrs.rename_all, field.span);

                // Merge flatten-site field_tags with parent's default_field_tags into one
                // defaults slice. Resolution order at read time:
                //   1. Child's own field-level tags (baked in child's static) win
                //   2. Child's own default_field_tag (baked in child's static) wins
                //   3. These merged defaults fill in for tag ids not already present
                // Within the merged defaults, flatten-site tags override parent defaults
                // (handled by resolve_field_tags precedence).
                let merged_defaults =
                    resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);

                // Compute the inflected prefix at macro time.
                // - Exact prefix: used as-is (e.g., exact_prefix("api") stays "api",
                //   prepended to the child's already-inflected field name).
                // - Inflectable prefix: requires a trailing delimiter (e.g., "api_").
                //   apply_prefix_only inflects it to the parent's style (e.g., PascalCase
                //   turns "api_" into "Api"). This works because the delimiter marks a
                //   word boundary, so inflecting prefix+base separately gives the same
                //   result as inflecting them together.
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
                // Compute name in all 4 styles
                field_names[0].push(metric_name(root_attrs, NameStyle::Preserve, field));
                field_names[1].push(metric_name(root_attrs, NameStyle::PascalCase, field));
                field_names[2].push(metric_name(root_attrs, NameStyle::SnakeCase, field));
                field_names[3].push(metric_name(root_attrs, NameStyle::KebabCase, field));

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
                    tags,
                    tags_ident,
                    unit_expr,
                });

                field_idx += 1;
            }
        }
    }

    let num_fields = field_infos.len();

    // Generate tag statics (shared across all 4 descriptors)
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

    // Generate 4 descriptor statics + trait impls
    let styles: [(&str, Ts2); 4] = [
        ("PRESERVE", quote! { ::metrique::Identity<__P> }),
        ("PASCAL", quote! { ::metrique::PascalCase<__P> }),
        ("SNAKE", quote! { ::metrique::SnakeCase<__P> }),
        ("KEBAB", quote! { ::metrique::KebabCase<__P> }),
    ];

    // Generate an inherent method on the entry type that selects the right
    // static descriptor based on the name style index. The compiler optimizes
    // the match away when called with a const index (which it always is).
    let style_arms: Vec<Ts2> = styles
        .iter()
        .enumerate()
        .map(|(style_idx, (style_name, _))| {
            let desc_ident = format_ident!("__METRIQUE_DESC_{}", style_name);
            let fields_ident = format_ident!("__METRIQUE_FIELDS_{}", style_name);
            let style_idx_u8 = style_idx as u8;

            let field_exprs: Vec<Ts2> = field_infos
                .iter()
                .enumerate()
                .map(|(fi_idx, fi)| {
                    let name = &field_names[style_idx][fi_idx];
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
                })
                .collect();

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
                            #timestamp_descriptor,
                        );
                    &#desc_ident
                }
            }
        })
        .collect();

    let (impl_generics_desc, ty_generics_desc, where_clause_desc) = generics.split_for_impl();

    let descriptor_impl = quote! {
        impl #impl_generics_desc #entry_name #ty_generics_desc #where_clause_desc {
            #[doc(hidden)]
            fn __metrique_descriptor(__style: u8) -> &'static ::metrique::writer::core::EntryDescriptor {
                match __style {
                    #(#style_arms)*
                    _ => unreachable!()
                }
            }
        }
    };

    // The struct's own descriptor uses its own rename_all (hardcoded at macro time).
    // NS::__STYLE_INDEX is only used when a parent calls this struct's descriptors()
    // via flatten (to propagate the parent's name style to this struct's fields).
    // But for the struct's own fields, the write path always uses make_ns(rename_all),
    // so the descriptor must match.
    let own_style_index = match root_attrs.rename_all {
        crate::inflect::NameStyle::Preserve => 0u8,
        crate::inflect::NameStyle::PascalCase => 1u8,
        crate::inflect::NameStyle::SnakeCase => 2u8,
        crate::inflect::NameStyle::KebabCase => 3u8,
    };

    let descriptors_method = quote! {
        fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
            #(#flatten_tag_statics)*
            ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                #entry_name::__metrique_descriptor(#own_style_index)
            ))
            #(#flatten_chains)*
        }
    };

    // Return trait impls (go outside InflectableEntry impl) and
    // descriptors method (goes inside InflectableEntry impl) separately.
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
