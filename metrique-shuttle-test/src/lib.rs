// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Internal, unpublished proc-macro crate: generates the `<name>_pct` /
//! `<name>_determinism` shuttle test pair every shuttle test in this
//! workspace needs, as an attribute instead of a separate macro call.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Expr, Ident, ItemFn, LitStr, Token, parse::Parse, parse::ParseStream, parse_macro_input,
    punctuated::Punctuated,
};

struct Args {
    num_iters: Expr,
    depth: Expr,
    should_panic: Option<LitStr>,
}

enum Field {
    NumIters(Expr),
    Depth(Expr),
    ShouldPanic(LitStr),
}

impl Parse for Field {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        match key.to_string().as_str() {
            "num_iters" => Ok(Field::NumIters(input.parse()?)),
            "depth" => Ok(Field::Depth(input.parse()?)),
            "should_panic" => Ok(Field::ShouldPanic(input.parse()?)),
            other => Err(syn::Error::new(
                key.span(),
                format!(
                    "unknown `shuttle_test` field `{other}` -- expected `num_iters`, `depth`, or `should_panic`"
                ),
            )),
        }
    }
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let fields = Punctuated::<Field, Token![,]>::parse_terminated(input)?;

        let mut num_iters = None;
        let mut depth = None;
        let mut should_panic = None;
        for field in fields {
            match field {
                Field::NumIters(v) => num_iters = Some(v),
                Field::Depth(v) => depth = Some(v),
                Field::ShouldPanic(v) => should_panic = Some(v),
            }
        }

        Ok(Args {
            num_iters: num_iters
                .ok_or_else(|| input.error("missing required `num_iters = <expr>`"))?,
            depth: depth.ok_or_else(|| input.error("missing required `depth = <expr>`"))?,
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
/// fn round_trip_no_loss() { /* ... */ }
/// ```
///
/// `num_iters` and `depth` are required, in either order. Add `,
/// should_panic = "..."` for tests expecting a panic.
#[proc_macro_attribute]
pub fn shuttle_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as Args);
    let item_fn = parse_macro_input!(item as ItemFn);

    let name = &item_fn.sig.ident;
    let pct_name = format_ident!("{name}_pct");
    let determinism_name = format_ident!("{name}_determinism");
    let iterations = &args.num_iters;
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
