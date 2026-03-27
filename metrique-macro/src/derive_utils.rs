use proc_macro2::TokenStream as Ts2;
use syn::{Attribute, spanned::Spanned};

pub(crate) fn extract_allowed_derives(attrs: &[Attribute]) -> Vec<Attribute> {
    let allowed = ["Debug", "Clone"];
    let mut found = vec![];

    for attr in attrs {
        if attr.path().is_ident("derive")
            && let syn::Meta::List(meta_list) = &attr.meta
        {
            let tokens = meta_list.tokens.to_string();
            for derive in &allowed {
                if tokens.contains(derive) {
                    found.push(syn::Ident::new(derive, meta_list.span()));
                }
            }
        }
    }

    if found.is_empty() {
        vec![]
    } else {
        vec![syn::parse_quote!(#[derive(#(#found),*)])]
    }
}

/// Strip specific derive traits from attributes, returning cleaned attributes.
/// Derives not in `to_strip` are preserved. Empty `#[derive()]` attrs are removed entirely.
pub(crate) fn strip_derives(attrs: &[Attribute], to_strip: &[&str]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter_map(|attr| {
            // Non derives passthrough, like #[docs(...)]
            if !attr.path().is_ident("derive") {
                return Some(attr.clone());
            }

            let syn::Meta::List(meta_list) = &attr.meta else {
                return Some(attr.clone());
            };
            let tokens = meta_list.tokens.to_string();
            let remaining: Vec<&str> = tokens
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !to_strip.iter().any(|d| s == d))
                .collect();

            // If no derives left, would be "derive()", so just remove it
            if remaining.is_empty() {
                None
            } else {
                let remaining_str = remaining.join(", ");
                let new_tokens: Ts2 = remaining_str.parse().unwrap();
                Some(syn::parse_quote!(#[derive(#new_tokens)]))
            }
        })
        .collect()
}

/// Returns the auto-derive attributes for value(string) enums: `#[derive(Debug, Clone, Copy)]`
///
/// These are always applied to the entry (Value) enum for `value(string)` enums,
/// regardless of what the user derives on the base enum.
pub(crate) fn value_string_auto_derives() -> Vec<Attribute> {
    let derives: Vec<_> = ["Debug", "Clone", "Copy"]
        .iter()
        .map(|d| syn::Ident::new(d, proc_macro2::Span::call_site()))
        .collect();
    vec![syn::parse_quote!(#[derive(#(#derives),*)])]
}
