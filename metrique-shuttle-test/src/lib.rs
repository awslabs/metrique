// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Internal, unpublished proc-macro crate: generates the `<name>_pct` /
//! `<name>_determinism` shuttle test pair every shuttle test in this
//! workspace needs, as an attribute instead of a separate macro call.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Expr, Ident, ItemFn, LitStr, Token, parse::Parse, parse::ParseStream, parse_macro_input,
};

struct Args {
    iterations: Expr,
    depth: Expr,
    should_panic: Option<LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let iterations: Expr = input.parse()?;
        input.parse::<Token![,]>()?;
        let depth: Expr = input.parse()?;

        let should_panic = if input.is_empty() {
            None
        } else {
            input.parse::<Token![,]>()?;
            let keyword: Ident = input.parse()?;
            if keyword != "should_panic" {
                return Err(syn::Error::new(keyword.span(), "expected `should_panic`"));
            }
            input.parse::<Token![=]>()?;
            Some(input.parse::<LitStr>()?)
        };

        Ok(Args {
            iterations,
            depth,
            should_panic,
        })
    }
}

/// Generates the `<name>_pct` / `<name>_determinism` shuttle test pair for
/// the function it's attached to, calling `shuttle::check_pct` and
/// `shuttle::check_uncontrolled_nondeterminism` with the same iteration
/// count -- the pattern every shuttle test in this workspace follows.
///
/// ```ignore
/// #[shuttle_test(2_000, 3)]
/// fn round_trip_no_loss() { /* ... */ }
/// ```
///
/// Add `, should_panic = "..."` for tests expecting a panic.
#[proc_macro_attribute]
pub fn shuttle_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let item_fn = parse_macro_input!(item as ItemFn);

    let name = &item_fn.sig.ident;
    let pct_name = format_ident!("{name}_pct");
    let determinism_name = format_ident!("{name}_determinism");
    let iterations = &args.iterations;
    let depth = &args.depth;

    let should_panic_attr = args.should_panic.as_ref().map(|msg| {
        quote! { #[should_panic(expected = #msg)] }
    });

    quote! {
        #item_fn

        #[test]
        #should_panic_attr
        fn #pct_name() {
            ::shuttle::check_pct(#name, #iterations, #depth);
        }

        #[test]
        #should_panic_attr
        fn #determinism_name() {
            ::shuttle::check_uncontrolled_nondeterminism(#name, #iterations);
        }
    }
    .into()
}
