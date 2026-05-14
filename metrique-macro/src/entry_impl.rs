// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module generates the implementation of the Entry trait for non-value structs and enums.
//! This gives us more control vs. `#[derive(Entry)]` over the generated code and improves compile-time errors.

use proc_macro2::TokenStream as Ts2;
use quote::{format_ident, quote, quote_spanned};
use syn::Ident;

use crate::{MetricsField, MetricsFieldKind, NameStyle, RootAttributes, inflect::metric_name};

mod enum_impl;
mod struct_impl;

pub(crate) use enum_impl::generate_enum_entry_impl;
pub(crate) use struct_impl::generate_struct_entry_impl;

use crate::FieldTagAttr;

/// Output of descriptor generation for a struct or enum entry.
pub(crate) struct DescriptorOutput {
    /// The `__metrique_descriptor(style)` inherent impl with 4 statics.
    /// Goes outside the `InflectableEntry` impl block but inside `const _: ()`.
    pub(crate) trait_impls: Ts2,
    /// The `fn descriptors()` method body.
    /// Goes inside the `InflectableEntry` impl block.
    pub(crate) method: Ts2,
}

/// Metadata for a single field in the descriptor, collected at macro time.
pub(crate) struct DescriptorFieldMeta {
    /// Field name in each style: [preserve, pascal, snake, kebab]
    pub(crate) names: [String; 4],
    /// Resolved flag token streams for this field
    pub(crate) flags: Vec<Ts2>,

    /// Unit expression (None or Some(<Unit>::UNIT))
    pub(crate) unit_expr: Ts2,
}

/// Generate a block that selects one of 4 pre-computed EntryDescriptor statics based on a style u8.
///
/// Returns a token stream like:
/// ```ignore
/// {
///     static FLAGS_0: [...] = [...];
///     match style {
///         STYLE_PRESERVE => { static FIELDS: [...]; static DESC: ...; &DESC }
///         ...
///     }
/// }
/// ```
pub(crate) fn generate_style_matched_descriptor(
    fields: &[DescriptorFieldMeta],
    desc_name: &str,
    timestamp_expr: &Ts2,
    ident_prefix: &str,
) -> Ts2 {
    let num_fields = fields.len();

    // Flag statics (shared across all 4 styles)
    let flag_statics: Vec<Ts2> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let flags_ident = format_ident!("__METRIQUE_{}_FLAGS_{}", ident_prefix, i);
            let flags = &f.flags;
            let num_flags = flags.len();
            quote! {
                static #flags_ident: [::metrique::writer::core::FieldFlag; #num_flags] = [
                    #(#flags),*
                ];
            }
        })
        .collect();

    let style_names = metrique_core::STYLE_NAMES;
    let style_arms: Vec<Ts2> = (0..style_names.len())
        .map(|style_idx| {
            let style_const = style_const_for(crate::inflect::NameStyle::ALL[style_idx]);
            let desc_ident = format_ident!("__METRIQUE_{}_DESC_{}", ident_prefix, style_names[style_idx]);
            let fields_ident = format_ident!("__METRIQUE_{}_FIELDS_{}", ident_prefix, style_names[style_idx]);

            let field_exprs: Vec<Ts2> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let name = &f.names[style_idx];
                    let flags_ident = format_ident!("__METRIQUE_{}_FLAGS_{}", ident_prefix, i);
                    let unit_expr = &f.unit_expr;
                    quote! {
                        ::metrique::writer::core::FieldDescriptor::builder(#name)
                            .flags(&#flags_ident)
                            .maybe_unit(#unit_expr)
                            .build()
                    }
                })
                .collect();

            quote! {
                #style_const => {
                    static #fields_ident: [::metrique::writer::core::FieldDescriptor; #num_fields] = [
                        #(#field_exprs),*
                    ];
                    static #desc_ident: ::metrique::writer::core::EntryDescriptor =
                        ::metrique::writer::core::EntryDescriptor::builder(#desc_name, &#fields_ident)
                            .maybe_timestamp(#timestamp_expr)
                            .build();
                    &#desc_ident
                }
            }
        })
        .collect();

    quote! {
        {
            #(#flag_statics)*
            match style {
                #(#style_arms),*
                _ => unreachable!()
            }
        }
    }
}

pub(crate) fn generate_descriptor_impl(
    entry_name: &Ident,
    generics: &syn::Generics,
    struct_name: &str,
    fields: &[DescriptorFieldMeta],
    timestamp_descriptor: &Ts2,
) -> Ts2 {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let body = generate_style_matched_descriptor(fields, struct_name, timestamp_descriptor, "");

    quote! {
        impl #impl_generics #entry_name #ty_generics #where_clause {
            #[doc(hidden)]
            #[inline(always)]
            fn __metrique_descriptor(__style: u8) -> &'static ::metrique::writer::core::EntryDescriptor {
                let style = __style;
                #body
            }
        }
    }
}

/// Returns the style constant token stream for a given `NameStyle`.
/// Maps the macro-internal `NameStyle` enum to the runtime `::metrique::STYLE_*` constants.
fn style_const_for(style: crate::inflect::NameStyle) -> Ts2 {
    match style {
        crate::inflect::NameStyle::Preserve => quote! { ::metrique::STYLE_PRESERVE },
        crate::inflect::NameStyle::PascalCase => quote! { ::metrique::STYLE_PASCAL },
        crate::inflect::NameStyle::SnakeCase => quote! { ::metrique::STYLE_SNAKE },
        crate::inflect::NameStyle::KebabCase => quote! { ::metrique::STYLE_KEBAB },
    }
}

/// Resolved flags and suppressed type ids for a field.
pub(crate) struct ResolvedFlags {
    pub(crate) flags: Vec<Ts2>,
}

pub(crate) fn resolve_field_flags(
    field_flags: &[FieldTagAttr],
    default_flags: &[FieldTagAttr],
) -> ResolvedFlags {
    let mut flags = Vec::new();

    // Field-level flags: present ones go to flags, skipped ones are just not emitted
    for flag in field_flags {
        let path = &flag.path;
        if flag.skip {
            // skip: don't emit this flag (it suppresses the default within this struct)
        } else {
            flags.push(quote! {
                ::metrique::writer::core::FieldFlag::new::<#path>()
            });
        }
    }

    // Default flags fill in for paths not already specified at field level
    for default_flag in default_flags {
        let path = &default_flag.path;
        let already_specified = field_flags.iter().any(|ft| ft.path == *path);
        if already_specified {
            continue;
        }
        if default_flag.skip {
            // skip on a default: don't emit
        } else {
            flags.push(quote! {
                ::metrique::writer::core::FieldFlag::new::<#path>()
            });
        }
    }

    ResolvedFlags { flags }
}

/// Hygiene helper for generated method-local identifiers.
///
/// When `#[metrics]` is expanded inside `macro_rules!`, field names from macro parameters
/// can have a different hygiene context than proc-macro-generated identifiers.
/// Using `Span::mixed_site()` keeps generated locals consistently resolvable in those bodies.
pub(crate) fn mixed_site_writer() -> Ident {
    format_ident!("writer", span = proc_macro2::Span::mixed_site())
}

/// Hygiene helper for the generated receiver binding (`__metrique_self`).
///
/// Similar to [`mixed_site_writer`], but specifically for `self` access:
/// the `self` keyword works in signatures/bindings, while `self.field` can fail across
/// hygiene boundaries. Generated code rebinds with `let __metrique_self = self;` and then
/// uses `__metrique_self.field` for field access.
pub(crate) fn mixed_site_self() -> Ident {
    format_ident!("__metrique_self", span = proc_macro2::Span::mixed_site())
}

fn make_ns(ns: NameStyle, span: proc_macro2::Span) -> Ts2 {
    match ns {
        NameStyle::PascalCase => quote_spanned! {span=> NS::PascalCase },
        NameStyle::SnakeCase => quote_spanned! {span=> NS::SnakeCase },
        NameStyle::KebabCase => quote_spanned! {span=> NS::KebabCase },
        NameStyle::Preserve => quote_spanned! {span=> NS },
    }
}

/// Generate a ConstStr struct with the given identifier and value.
/// Used to create compile-time constant strings for metric names and prefixes.
fn const_str(ident: &syn::Ident, value: &str) -> Ts2 {
    quote_spanned! {ident.span()=>
        struct #ident;
        impl ::metrique::concat::ConstStr for #ident {
            const VAL: &'static str = #value;
        }
    }
}

/// Generate 4 ConstStr structs (one per naming style) and build an Inflect namespace type.
/// The `name_fn` callback computes the string value for each style.
/// Returns (extra_code, inflected_type).
fn make_inflect_base(
    ns: &Ts2,
    inflect_method: syn::Ident,
    span: proc_macro2::Span,
    mut name_fn: impl FnMut(NameStyle) -> String,
) -> (Ts2, Ts2) {
    let preserve_val = name_fn(NameStyle::Preserve);
    let kebab_val = name_fn(NameStyle::KebabCase);
    let pascal_val = name_fn(NameStyle::PascalCase);
    let snake_val = name_fn(NameStyle::SnakeCase);

    // Sanitize to create valid Rust identifiers, applying PascalCase explicitly rather than via
    // name_fn (to overwrite even `name` attributes)
    let ident_base: String = NameStyle::PascalCase
        .apply(&preserve_val)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();

    let name_ident = format_ident!(
        "{}{}",
        ident_base,
        NameStyle::Preserve.to_word(),
        span = span
    );
    let name_kebab = format_ident!(
        "{}{}",
        ident_base,
        NameStyle::KebabCase.to_word(),
        span = span
    );
    let name_pascal = format_ident!(
        "{}{}",
        ident_base,
        NameStyle::PascalCase.to_word(),
        span = span
    );
    let name_snake = format_ident!(
        "{}{}",
        ident_base,
        NameStyle::SnakeCase.to_word(),
        span = span
    );

    let extra_preserve = const_str(&name_ident, &preserve_val);
    let extra_kebab = const_str(&name_kebab, &kebab_val);
    let extra_pascal = const_str(&name_pascal, &pascal_val);
    let extra_snake = const_str(&name_snake, &snake_val);

    let extra = quote!(
        #extra_preserve
        #extra_kebab
        #extra_pascal
        #extra_snake
    );

    let inflected_type = quote!(
        <#ns as ::metrique::NameStyle>::#inflect_method<#name_ident, #name_pascal, #name_snake, #name_kebab>
    );

    (extra, inflected_type)
}

/// Generate inflectable name using the `Inflect` method.
/// Creates 4 ConstStr structs and returns a namespace type that selects the appropriate variant.
fn make_inflect(
    ns: &Ts2,
    span: proc_macro2::Span,
    name_fn: impl FnMut(NameStyle) -> String,
) -> (Ts2, Ts2) {
    make_inflect_base(ns, format_ident!("Inflect", span = span), span, name_fn)
}

/// Generate inflectable affix using the `InflectAffix` method.
/// Creates 4 ConstStr structs and returns a namespace type that selects the appropriate variant.
/// Note: This does not append the prefix from `ns` as per the behavior of `InflectAffix`.
fn make_inflect_affix(
    ns: &Ts2,
    span: proc_macro2::Span,
    name_fn: impl FnMut(NameStyle) -> String,
) -> (Ts2, Ts2) {
    make_inflect_base(
        ns,
        format_ident!("InflectAffix", span = span),
        span,
        name_fn,
    )
}

/// Generate an inflectable prefix that adapts to the namespace style.
/// Creates 4 ConstStr structs (preserve, pascal, snake, kebab) and returns
/// a namespace type that selects the appropriate variant via InflectAffix.
/// Returns (extra_code, namespace_with_prefix).
pub(crate) fn make_inflect_prefix(ns: &Ts2, prefix: &str, span: proc_macro2::Span) -> (Ts2, Ts2) {
    let (extra, inflected) = make_inflect_affix(ns, span, |style| style.apply_prefix(prefix));

    let ns_with_prefix = quote!(
        <#ns as ::metrique::NameStyle>::AppendPrefix<#inflected>
    );

    (extra, ns_with_prefix)
}

/// Generate an exact (non-inflectable) prefix that never changes.
/// Creates 1 ConstStr struct and returns a namespace type with the prefix applied.
/// Returns (extra_code, namespace_with_prefix).
pub(crate) fn make_exact_prefix(
    ns: &Ts2,
    exact_prefix: &str,
    span: proc_macro2::Span,
) -> (Ts2, Ts2) {
    // Apply PascalCase first, then sanitize to create a valid identifier
    let pascal_val = NameStyle::PascalCase.apply(exact_prefix);
    let ident_base: String = pascal_val.chars().filter(|c| c.is_alphanumeric()).collect();
    let prefix_ident = format_ident!("{}Preserve", ident_base, span = span);
    let extra = const_str(&prefix_ident, exact_prefix);
    let ns_with_prefix = quote!(
        <#ns as ::metrique::NameStyle>::AppendPrefix<#prefix_ident>
    );
    (extra, ns_with_prefix)
}

fn generate_field_writes(
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
    field_access: impl Fn(&Ts2) -> Ts2,
) -> Vec<Ts2> {
    let mut writes = Vec::new();
    let writer_ident = mixed_site_writer();

    for field in fields {
        let field_span = field.span;
        let ns = make_ns(root_attrs.rename_all, field_span);
        let cfg_attrs: Vec<_> = field.cfg_attrs().collect();

        let write = match &field.attrs.kind {
            MetricsFieldKind::Timestamp(span) => {
                let field_access = field_access(&field.ident);
                quote_spanned! {*span=>
                    #[allow(clippy::useless_conversion)]
                    {
                        ::metrique::writer::EntryWriter::timestamp(#writer_ident, (*#field_access).into());
                    }
                }
            }
            MetricsFieldKind::FlattenEntry(span) => {
                let field_access = field_access(&field.ident);
                quote_spanned! {*span=>
                    ::metrique::writer::Entry::write(#field_access, #writer_ident);
                }
            }
            MetricsFieldKind::Flatten { span, prefix } => {
                let (extra, ns) = match prefix {
                    None => (quote!(), ns),
                    Some(prefix) => prefix.append_to(&ns, field_span),
                };
                let field_access = field_access(&field.ident);
                quote_spanned! {*span=>
                    #extra
                    ::metrique::InflectableEntry::<#ns>::write(#field_access, #writer_ident);
                }
            }
            MetricsFieldKind::Ignore(_) => {
                continue;
            }
            MetricsFieldKind::Field { format, .. } => {
                let (extra, name) = make_inflect_metric_name(root_attrs, field);
                let field_access = field_access(&field.ident);
                let value = crate::value_impl::format_value(format, field_span, field_access);

                // Wrap value in ForceFlag for each present flag (non-skip field-level
                // and non-skip defaults not overridden at field level)
                let present_flags: Vec<_> =
                    resolve_field_flags(&field.attrs.flags, &root_attrs.default_flags).flags;
                let wrapped_value = if present_flags.is_empty() {
                    quote! { #value }
                } else {
                    // Collect the FlagConstructor paths (non-skip)
                    let flag_paths: Vec<_> = field
                        .attrs
                        .flags
                        .iter()
                        .filter(|f| !f.skip)
                        .map(|f| &f.path)
                        .chain(
                            root_attrs
                                .default_flags
                                .iter()
                                .filter(|d| {
                                    !d.skip && !field.attrs.flags.iter().any(|ft| ft.path == d.path)
                                })
                                .map(|d| &d.path),
                        )
                        .collect();
                    let mut expr = quote! { #value };
                    for path in &flag_paths {
                        expr = quote! {
                            ::metrique::writer::value::ForceFlag::<_, #path>::from(#expr)
                        };
                    }
                    quote! { &#expr }
                };

                quote_spanned! {field_span=>
                    ::metrique::writer::EntryWriter::value(#writer_ident,
                        {
                            #extra
                            ::metrique::concat::const_str_value::<#name>()
                        }
                        , #wrapped_value);
                }
            }
        };
        if cfg_attrs.is_empty() {
            writes.push(write);
        } else {
            writes.push(quote! { #(#cfg_attrs)* { #write } });
        }
    }

    writes
}

pub(crate) fn make_binary_tree_chain(iterators: Vec<Ts2>) -> Ts2 {
    if iterators.is_empty() {
        return quote! { ::std::iter::empty() };
    }

    if iterators.len() == 1 {
        return iterators[0].clone();
    }

    // Split the iterators in half and recursively build the tree
    let mid = iterators.len() / 2;
    let left = make_binary_tree_chain(iterators[..mid].to_vec());
    let right = make_binary_tree_chain(iterators[mid..].to_vec());

    quote! { #left.chain(#right) }
}

fn make_inflect_metric_name(root_attrs: &RootAttributes, field: &MetricsField) -> (Ts2, Ts2) {
    make_inflect(
        &make_ns(root_attrs.rename_all, field.span),
        field.span,
        |style| metric_name(root_attrs, style, field),
    )
}

/// Collect sample group iterators from a field, returning (field_ident, iterator_expr) for fields that have sample groups.
/// The `field_access` closure determines how to access the field (e.g., `#field_ident` or `&__metrique_self.#field_ident`).
///
/// The returned iterator expression is guarded with the field's cfg/cfg_attr attributes:
/// it starts from `empty()` and conditionally chains the field iterator when the attrs apply.
/// This avoids referencing cfg-disabled fields and works for both `cfg(...)` and
/// `cfg_attr(..., cfg(...))` forms without re-implementing cfg predicate logic.
fn collect_field_sample_group<'a>(
    field: &'a MetricsField,
    root_attrs: &RootAttributes,
    field_access: impl FnOnce(&Ts2) -> Ts2,
) -> Option<(&'a Ts2, Ts2)> {
    let field_ident = &field.ident;
    let cfg_attrs: Vec<_> = field.cfg_attrs().collect();
    let inner = match &field.attrs.kind {
        MetricsFieldKind::Flatten { span, .. } => {
            let ns = make_ns(root_attrs.rename_all, field.span);
            let access = field_access(field_ident);
            quote_spanned!(*span=>
                ::metrique::InflectableEntry::<#ns>::sample_group(#access)
            )
        }
        MetricsFieldKind::FlattenEntry(span) => {
            let access = field_access(field_ident);
            quote_spanned!(*span=>
                ::metrique::writer::Entry::sample_group(#access)
            )
        }
        MetricsFieldKind::Field {
            sample_group: Some(span),
            ..
        } => {
            let (extra, name) = make_inflect_metric_name(root_attrs, field);
            let access = field_access(field_ident);
            quote_spanned!(*span=>
                {
                    #extra
                    ::std::iter::once((
                        ::metrique::concat::const_str_value::<#name>(),
                        ::metrique::writer::core::SampleGroup::as_sample_group(#access)
                    ))
                }
            )
        }
        MetricsFieldKind::Field {
            sample_group: None, ..
        }
        | MetricsFieldKind::Ignore(_)
        | MetricsFieldKind::Timestamp(_) => return None,
    };
    if cfg_attrs.is_empty() {
        Some((field_ident, inner))
    } else {
        let wrapped = quote! {
            {
                let __metrique_sg = ::std::iter::empty::<(
                    ::std::borrow::Cow<'static, str>,
                    ::std::borrow::Cow<'static, str>,
                )>();
                #(#cfg_attrs)*
                let __metrique_sg = __metrique_sg.chain(#inner);
                __metrique_sg
            }
        };
        Some((field_ident, wrapped))
    }
}
