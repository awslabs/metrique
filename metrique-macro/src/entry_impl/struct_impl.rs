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
    let descriptor_fn = generate_descriptor(entry_name, fields, root_attrs);

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
            #(#writes)*
        }
    };

    let sample_group_fn = quote_spanned! {mixed=>
        fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
            let #self_ident = self;
            #sample_groups
        }
    };

    // we generate one entry impl for each namestyle. This will then allow the parent to
    // transitively set the namestyle
    quote! {
        const _: () = {
            #[expect(deprecated)]
            impl #impl_generics ::metrique::InflectableEntry<NS> for #entry_name #ty_generics #where_clause {
                #write_fn
                #sample_group_fn
                #descriptor_fn
            }
        };
    }
}

fn generate_descriptor(
    entry_name: &Ident,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();

    let mut tag_statics = Vec::new();
    let mut field_descriptors = Vec::new();
    let mut timestamp_descriptor = quote! { None };
    let mut field_idx = 0usize;
    let mut flatten_chains = Vec::new();

    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Ignore(_) => continue,
            MetricsFieldKind::Timestamp(_) => {
                let name = field.name.as_deref().unwrap_or("timestamp");
                timestamp_descriptor = quote! {
                    Some(::metrique::writer::core::TimestampDescriptor::__metrique_private_new(#name))
                };
            }
            MetricsFieldKind::Flatten { .. } => {
                let field_ident = &field.ident;
                flatten_chains.push(quote! {
                    .chain(::metrique::InflectableEntry::<NS>::descriptors(&self.#field_ident))
                });
            }
            MetricsFieldKind::FlattenEntry(_) => {
                let field_ident = &field.ident;
                flatten_chains.push(quote! {
                    .chain(::metrique::writer::Entry::descriptors(&self.#field_ident))
                });
            }
            MetricsFieldKind::Field { unit, .. } => {
                let field_name = metric_name(root_attrs, root_attrs.rename_all, field);

                let tags =
                    resolve_field_tags(&field.attrs.field_tags, &root_attrs.default_field_tags);
                let num_tags = tags.len();
                let tags_ident = format_ident!("__METRIQUE_TAGS_{}", field_idx);

                tag_statics.push(quote! {
                    static #tags_ident: [::metrique::writer::core::ResolvedFieldTag; #num_tags] = [
                        #(#tags),*
                    ];
                });

                let unit_expr = match unit {
                    Some(u) => {
                        quote! { Some(<#u as ::metrique::writer::core::unit::UnitTag>::UNIT) }
                    }
                    None => quote! { None },
                };

                field_descriptors.push(quote! {
                    ::metrique::writer::core::FieldDescriptor::__metrique_private_new(
                        #field_name,
                        &#tags_ident,
                        ::metrique::writer::core::FieldShape::Opaque,
                        #unit_expr,
                    )
                });

                field_idx += 1;
            }
        }
    }

    let num_fields = field_descriptors.len();

    quote! {
        fn descriptors(&self) -> impl ::std::iter::Iterator<Item = ::metrique::writer::core::DescriptorRef<'_>> {
            #(#tag_statics)*
            static __METRIQUE_FIELDS: [::metrique::writer::core::FieldDescriptor; #num_fields] = [
                #(#field_descriptors),*
            ];
            static __METRIQUE_DESCRIPTOR: ::metrique::writer::core::EntryDescriptor =
                ::metrique::writer::core::EntryDescriptor::__metrique_private_new(
                    #struct_name,
                    &__METRIQUE_FIELDS,
                    #timestamp_descriptor,
                );
            ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(&__METRIQUE_DESCRIPTOR))
                #(#flatten_chains)*
        }
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
