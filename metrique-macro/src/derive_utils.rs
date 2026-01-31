use proc_macro2::TokenStream as Ts2;
use quote::quote;
use syn::{Attribute, spanned::Spanned};

pub(crate) fn extract_allowed_derives(attrs: &[Attribute]) -> Ts2 {
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
        quote!()
    } else {
        quote!(#[derive(#(#found),*)])
    }
}
