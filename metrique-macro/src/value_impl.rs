use crate::{MetricsField, MetricsFieldKind, NameStyle, RootAttributes, enums::MetricsVariant};

use proc_macro2::{Span, TokenStream as Ts2};
use quote::{quote, quote_spanned};
use syn::Ident;

pub(crate) fn generate_value_impl_for_enum(
    root_attrs: &RootAttributes,
    value_name: &Ident,
    parsed_variants: &[MetricsVariant],
) -> Ts2 {
    let from_and_sample_group = crate::enums::generate_from_and_sample_group_for_enum(
        value_name,
        parsed_variants,
        root_attrs,
    );

    quote!(
        #from_and_sample_group
        impl ::metrique::writer::Value for #value_name {
            fn write(&self, writer: impl ::metrique::writer::ValueWriter) {
                writer.string(::std::convert::Into::<&str>::into(self));
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
    for field in &non_ignore_fields {
        if let MetricsFieldKind::Field {
            unit: _,
            sample_group,
            name,
            format: _,
        } = &field.attrs.kind
        {
            if sample_group.is_some() {
                return Err(syn::Error::new(
                    field.span,
                    "`sample_group` in value structs is used as a struct attribute, not a field attribute: `#[metrics(value, sample_group)]`",
                ));
            }
            if name.is_some() {
                return Err(syn::Error::new(
                    field.span,
                    "`name` does not make sense with #[metrics(value)]",
                ));
            }
        }
    }
    if root_attrs.sample_group && non_ignore_fields.is_empty() {
        return Err(syn::Error::new(
            value_name.span(),
            "`sample_group` requires a non-ignore field",
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

pub(crate) fn format_value(format: &Option<syn::Path>, span: Span, field: Ts2) -> Ts2 {
    if let Some(format) = format {
        quote_spanned! { span=> &::metrique::format::FormattedValue::<_, #format, _>::new(#field)}
    } else {
        field
    }
}

pub(crate) fn generate_value_impl_for_struct(
    root_attrs: &RootAttributes,
    value_name: &Ident,
    parsed_fields: &[MetricsField],
) -> Result<Ts2, syn::Error> {
    // support struct with only ignored fields as no value for orthogonality
    let mut non_ignore_fields_iter = parsed_fields
        .iter()
        .filter(|f| !matches!(f.attrs.kind, MetricsFieldKind::Ignore(_)));
    let non_ignore_field = non_ignore_fields_iter.next();
    assert!(
        non_ignore_fields_iter.next().is_none(),
        "value impl can't have multiple non-ignore fields"
    );
    let (body, sample_group_impl) = non_ignore_field
        .map(|field| match &field.attrs.kind {
            MetricsFieldKind::Field {
                unit: _,
                sample_group: _,
                name: _,
                format,
            } => {
                let ident = &field.ident;
                let value = format_value(
                    format,
                    field.span,
                    quote_spanned! {field.span=> &self.#ident },
                );
                let sample_group_impl = if root_attrs.sample_group {
                    // SampleGroup impl is only valid if there is a field
                    quote_spanned! {field.span=>
                        impl ::metrique::writer::core::SampleGroup for #value_name {
                            fn as_sample_group(&self) -> ::std::borrow::Cow<'static, str> {
                                #[allow(deprecated)] {
                                    ::metrique::writer::core::SampleGroup::as_sample_group(&self.#ident)
                                }
                            }
                        }
                    }
                } else {
                    quote!{}
                };
                Ok((quote_spanned! {field.span=> ::metrique::writer::Value::write(#value, writer); }, sample_group_impl))
            }
            _ => Err(syn::Error::new(
                field.span,
                "only plain fields are supported in #[metrics(value)]",
            )),
        })
        .transpose()?.unzip();
    Ok(quote! {
        impl ::metrique::writer::Value for #value_name {
            fn write(&self, writer: impl ::metrique::writer::ValueWriter) {
                #[allow(deprecated)] {
                    #body
                }
            }
        }

        #sample_group_impl
    })
}
