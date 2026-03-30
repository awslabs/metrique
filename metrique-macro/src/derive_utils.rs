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

/// Returns the auto-derive attributes for value(string) enums: `#[derive(Debug, Clone, Copy)]`
///
/// These are always applied to the generated Value enum for `value(string)` enums.
pub(crate) fn value_string_auto_derives() -> Vec<Attribute> {
    let derives: Vec<_> = ["Debug", "Clone", "Copy"]
        .iter()
        .map(|d| syn::Ident::new(d, proc_macro2::Span::call_site()))
        .collect();
    vec![syn::parse_quote!(#[derive(#(#derives),*)])]
}
