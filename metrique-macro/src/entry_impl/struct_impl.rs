use super::*;

pub(crate) fn generate_struct_entry_impl(
    entry_name: &Ident,
    generics: &syn::Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let writes = generate_write_statements(fields, root_attrs);
    let sample_groups = generate_sample_group_statements(fields, root_attrs);

    // Add NS as an additional generic parameter
    let mut impl_generics = generics.clone();
    impl_generics
        .params
        .push(syn::parse_quote!(NS: ::metrique::NameStyle));
    let (impl_generics, _, _) = impl_generics.split_for_impl();
    let (_, ty_generics, where_clause) = generics.split_for_impl();

    // we generate one entry impl for each namestyle. This will then allow the parent to
    // transitively set the namestyle
    quote! {
        const _: () = {
            #[expect(deprecated)]
            impl #impl_generics ::metrique::InflectableEntry<NS> for #entry_name #ty_generics #where_clause {
                fn write<'__metrique_write>(&'__metrique_write self, writer: &mut impl ::metrique::writer::EntryWriter<'__metrique_write>) {
                    #(#writes)*
                }

                fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                    #sample_groups
                }
            }
        };
    }
}

fn generate_write_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Vec<Ts2> {
    let mut writes = Vec::new();

    for field_ident in root_attrs.configuration_field_names() {
        writes.push(quote! {
            ::metrique::writer::Entry::write(&self.#field_ident, writer);
        });
    }

    writes.extend(generate_field_writes(
        fields,
        root_attrs,
        |field_ident| quote! { &self.#field_ident },
    ));
    writes
}

fn generate_sample_group_statements(fields: &[MetricsField], root_attrs: &RootAttributes) -> Ts2 {
    let sample_group_fields: Vec<_> = fields
        .iter()
        .filter_map(|field| {
            collect_field_sample_group(field, root_attrs, |f| quote! { &self.#f })
                .map(|(_, iter)| iter)
        })
        .collect();

    make_binary_tree_chain(sample_group_fields)
}
