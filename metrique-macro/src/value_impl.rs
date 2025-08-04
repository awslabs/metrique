use crate::{
    MetricsField, MetricsFieldKind, MetricsVariant, NameStyle, RootAttributes, metric_name,
};

use proc_macro2::TokenStream as Ts2;
use quote::{quote, quote_spanned};
use syn::Ident;

pub(crate) fn generate_value_impl_for_enum(
    root_attrs: &RootAttributes,
    value_name: &Ident,
    parsed_variants: &[MetricsVariant],
) -> Ts2 {
    let variants_and_strings = parsed_variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        let metric_name = metric_name(root_attrs, root_attrs.rename_all, variant);
        quote_spanned!(variant.ident.span()=> #value_name::#variant_ident => #metric_name)
    });
    quote!(
        impl ::metrique::__writer::Value for #value_name {
            fn write(&self, writer: impl ::metrique::__writer::ValueWriter) {
                writer.string(#[allow(deprecated)] match self {
                    #(#variants_and_strings),*
                });
            }
        }
    )
}

pub fn validate_value_impl_for_struct(
    root_attrs: &RootAttributes,
    value_name: &Ident,
    parsed_fields: &[MetricsField],
) -> Result<(), syn::Error> {
    let non_ignore_fields: Vec<&MetricsField> = parsed_fields
        .iter()
        .filter(|f| !matches!(f.attrs.kind, MetricsFieldKind::Ignore(_)))
        .collect::<Vec<_>>();
    if non_ignore_fields.len() > 1 {
        return Err(syn::Error::new(
            non_ignore_fields[1].span,
            "multiple non-ignored fields for #[metrics(value)]",
        ));
    }
    if root_attrs.emf_dimensions.is_some() {
        return Err(syn::Error::new(
            value_name.span(),
            "emf_dimensions is not supported for #[metrics(value)]",
        ));
    }
    if root_attrs.prefix.is_some() {
        return Err(syn::Error::new(
            value_name.span(),
            "prefix is not supported for #[metrics(value)]",
        ));
    }
    if !matches!(root_attrs.rename_all, NameStyle::Preserve) {
        return Err(syn::Error::new(
            value_name.span(),
            "NameStyle is not supported for #[metrics(value)]",
        ));
    }

    Ok(())
}

pub(crate) fn generate_value_impl_for_struct(
    _root_attrs: &RootAttributes,
    value_name: &Ident,
    parsed_fields: &[MetricsField],
) -> Result<Ts2, syn::Error> {
    // support struct with only ignored fields as no value for orthogonality
    let non_ignore_fields = parsed_fields
        .iter()
        .filter(|f| !matches!(f.attrs.kind, MetricsFieldKind::Ignore(_)));
    let body: Vec<Ts2> = non_ignore_fields.map(|field| {
        match field.attrs.kind {
            MetricsFieldKind::Field { unit: None, name: None, format: None } => {
                let ident = &field.ident;
                Ok(quote_spanned!{field.span=> ::metrique::__writer::Value::write(&self.#ident, writer) })
            }
            _ => {
                Err(syn::Error::new(field.span, "only plain fields are supported in #[metrics(value)]"))
            }
        }
    }).collect::<Result<Vec<_>, _>>()?;

    Ok(quote! {
        impl ::metrique::__writer::Value for #value_name {
            fn write(&self, writer: impl ::metrique::__writer::ValueWriter) {
                #[allow(deprecated)] {
                    #(#body);*
                }
            }
        }
    })
}
