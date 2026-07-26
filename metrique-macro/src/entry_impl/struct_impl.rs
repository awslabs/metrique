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
/// Own (non-flatten) fields are split into contiguous runs at flatten
/// boundaries so that `descriptors()` yields segments in the same order
/// `Entry::write` emits values: each run of own fields becomes its own
/// segment, interleaved with the flattened children's segments at their
/// declaration position.
///
/// Run 0 is always emitted (even when empty) so the first segment carries the
/// entry's canonical name and timestamp; later runs are emitted only when
/// non-empty.
fn generate_descriptor(
    entry_name: &Ident,
    generics: &syn::Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> super::DescriptorOutput {
    let struct_name = entry_name.to_string().trim_end_matches("Entry").to_string();
    let mut timestamp_descriptor = quote! { None };
    let styles = NameStyle::ALL;
    let own_style_ns = make_ns(root_attrs.rename_all, entry_name.span());

    // Own-field runs split at flatten boundaries, and the chain items
    // (own-run segments and flatten children) following the base segment,
    // in declaration order.
    let mut runs: Vec<Vec<DescriptorFieldMeta>> = vec![Vec::new()];
    let mut chain_items: Vec<(Vec<Ts2>, Ts2)> = Vec::new();

    for field in fields {
        match &field.attrs.kind {
            MetricsFieldKind::Ignore(_) => continue,
            MetricsFieldKind::Timestamp(_) => {
                let name = field.name.as_deref().unwrap_or("timestamp");
                timestamp_descriptor = quote! {
                    Some(::metrique::writer::core::TimestampDescriptor::new(#name))
                };
            }
            MetricsFieldKind::Field { unit, format, .. } => {
                let names: [String; metrique_core::Styles::COUNT] =
                    std::array::from_fn(|i| metric_name(root_attrs, styles[i], field));
                let resolved = resolve_field_flags(&field.attrs.flags, &root_attrs.default_flags);
                runs.last_mut()
                    .expect("runs is never empty")
                    .push(DescriptorFieldMeta {
                        names,
                        flags: resolved.flags,
                        skipped_flags: resolved.skipped_flags,
                        explicit_unit: unit.clone(),
                        field_type: field.ty.clone(),
                        close: field.attrs.close,
                        format: format.clone(),
                    });
            }
            MetricsFieldKind::Flatten { .. } | MetricsFieldKind::FlattenEntry(_) => {
                // Close the current run; non-base runs chain in just before
                // this flatten's child segments.
                let run_index = runs.len() - 1;
                if run_index > 0 && !runs[run_index].is_empty() {
                    chain_items.push((
                        vec![],
                        own_run_segment_expr(entry_name, &own_style_ns, run_index),
                    ));
                }
                runs.push(Vec::new());
                chain_items.push(flatten_chain_item(field, root_attrs));
            }
        }
    }
    // Close the trailing run.
    let last_run = runs.len() - 1;
    if last_run > 0 && !runs[last_run].is_empty() {
        chain_items.push((
            vec![],
            own_run_segment_expr(entry_name, &own_style_ns, last_run),
        ));
    }

    let descriptor_impl = generate_descriptor_impl(
        entry_name,
        generics,
        &struct_name,
        &runs,
        &timestamp_descriptor,
    );

    let descriptors_method = assemble_descriptors_method(entry_name, &own_style_ns, &chain_items);

    super::DescriptorOutput {
        trait_impls: descriptor_impl,
        method: descriptors_method,
    }
}

/// A `Descriptors` expression yielding the segment for one of the entry's own
/// field runs.
fn own_run_segment_expr(entry_name: &Ident, own_style_ns: &Ts2, run: usize) -> Ts2 {
    let method = super::descriptor_method_ident(run);
    quote! {
        ::metrique::writer::core::Descriptors::available(
            ::std::iter::once(::metrique::writer::core::DescriptorRef::from_static(
                #entry_name::#method(),
                <#own_style_ns as ::metrique::NameStyle>::DESCRIPTOR_STYLE_INDEX,
            ))
        )
    }
}

/// Builds the chain entry for a single flatten field in the `descriptors()`
/// method.
///
/// Returns `(cfg_attrs, expr)` where `expr` yields the child's descriptor
/// segments with any flatten-site modifiers (prefix, default_flags) applied.
/// Cfg-gated fields are chained via let-rebinding by `build_descriptors_chain`.
fn flatten_chain_item(field: &MetricsField, root_attrs: &RootAttributes) -> (Vec<Ts2>, Ts2) {
    let field_ident = &field.ident;
    let ns = make_ns(root_attrs.rename_all, field.span);
    let binding = quote! { &self.#field_ident };
    let child_expr = super::flatten_descriptors_expr(&field.attrs.kind, &binding, &ns);
    let cfg_attrs = field.cfg_attrs().map(|a| quote! { #a }).collect();
    (cfg_attrs, child_expr)
}

/// Assembles the `descriptors()` method body from the entry's base segment
/// (own-field run 0) and the interleaved chain items (later own-field runs and
/// flatten children, in declaration order).
///
/// When all chain items are non-cfg, generates a simple expression chain.
/// When cfg-gated items exist, uses let-rebinding so cfg-disabled fields
/// are excluded without affecting the iterator type.
fn assemble_descriptors_method(
    entry_name: &Ident,
    own_style_ns: &Ts2,
    chain_items: &[(Vec<Ts2>, Ts2)],
) -> Ts2 {
    let base_expr = own_run_segment_expr(entry_name, own_style_ns, 0);
    let chain_expr = super::build_descriptors_chain(base_expr, chain_items);

    quote! {
        fn descriptors(&self) -> ::metrique::writer::core::Descriptors<'_> {
            #chain_expr
        }
    }
}
