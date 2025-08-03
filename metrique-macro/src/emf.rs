// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use darling::FromMeta;
use proc_macro2::TokenStream as Ts2;
use quote::{ToTokens, quote};
use syn::{Expr, spanned::Spanned};

#[derive(Debug, Clone)]
pub(crate) struct DimensionSet {
    pub(crate) dimensions: Vec<Expr>,
}

impl DimensionSet {
    fn from_expr(expr: &syn::Expr) -> darling::Result<Self> {
        match expr {
            syn::Expr::Array(array) => Ok(DimensionSet {
                dimensions: array.elems.iter().cloned().collect(),
            }),
            _other => Err(darling::Error::custom(
                r#"expected a list of dimensions like `["Service", "Operation"]`"#,
            )
            .with_span(&expr.span())),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DimensionSets {
    pub(crate) sets: Vec<DimensionSet>,
}

impl FromMeta for DimensionSets {
    fn from_expr(expr: &syn::Expr) -> darling::Result<Self> {
        match expr {
            syn::Expr::Array(array) => {
                let mut accum = darling::Error::accumulator();
                let sets = array
                    .elems
                    .iter()
                    .flat_map(|expr| accum.handle(DimensionSet::from_expr(expr)))
                    .collect();
                accum.finish_with(DimensionSets { sets })
            }
            _other => Err(darling::Error::custom(
                "Expected a nested array of strings [[\"a\"], [\"b\"]]",
            )
            .with_span(&expr.span())),
        }
    }
}

impl ToTokens for DimensionSet {
    fn to_tokens(&self, tokens: &mut Ts2) {
        let dimensions = &self.dimensions;
        quote! { [#(#dimensions),*]}.to_tokens(tokens)
    }
}

impl ToTokens for DimensionSets {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let sets = &self.sets;
        quote! { [#(#sets),*]}.to_tokens(tokens)
    }
}
