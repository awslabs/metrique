// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod emf;
mod entry_impl;

use darling::{FromField, FromMeta, ast::NestedMeta, util::Flag};
use emf::{DimensionSets, NameStyle};
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Ts2};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, Generics, Ident, Result, Type, Visibility,
    parse_macro_input, spanned::Spanned,
};

/// Transforms a struct into a unit-of-work metric.
///
/// # Container Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `rename_all` | String | Changes the case style of all field names | `#[metrics(rename_all = "PascalCase")]` |
/// | `prefix` | String | Adds a prefix to all field names | `#[metrics(prefix = "api_")]` |
/// | `emf::dimension_sets` | Array | Defines dimension sets for CloudWatch metrics | `#[metrics(emf::dimension_sets = [["Status", "Operation"]])]` |
/// | subfield | Flag | When set, this metric can only be used when nested within other metrics. It cannot be added to a sink directly. | `#[metrics(subfield)]` |
///
/// # Field Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `name` | String | Overrides the field name in metrics | `#[metrics(name = "CustomName")]` |
/// | `unit` | Path | Specifies the unit for the metric value | `#[metrics(unit = Millisecond)]` |
/// | `timestamp` | Flag | Marks a field as the canonical timestamp | `#[metrics(timestamp)]` |
/// | `flatten` | Flag | Flattens nested `CloseValue` metric structs | `#[metrics(flatten)]` |
/// | `flatten_entry` | Flag | Flattens nested `Entry` metric structs | `#[metrics(flatten_entry)]` |
/// | `ignore` | Flag | Excludes the field from metrics | `#[metrics(ignore)]` |
///
/// # Example
///
/// ```rust,ignore
/// use metrique::unit_of_work::metrics;
/// use metrique::timers::{Timestamp, Timer};
/// use metrique::unit::Millisecond;
/// use metrique_writer::{GlobalEntrySink, ServiceMetrics};
/// use std::time::SystemTime;
///
/// #[metrics(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     operation: &'static str,
///
///     #[metrics(timestamp)]
///     timestamp: SystemTime,
///
///     #[metrics(unit = Millisecond)]
///     operation_time: Timer,
///
///     #[metrics(flatten)]
///     nested: NestedMetrics,
///
///     request_count: usize,
/// }
///
/// #[metrics(subfield, prefix = "sub_")]
/// struct NestedMetrics {
///     #[metrics(name = "CustomCounter")]
///     counter: usize,
/// }
///
/// impl RequestMetrics {
///     fn init(operation: &'static str) -> RequestMetricsGuard {
///         RequestMetrics {
///             timestamp: SystemTime::now(),
///             operation,
///             operation_time: Timer::start_now(),
///             nested: NestedMetrics { counter: 0 },
///             request_count: 0,
///         }.append_on_drop(ServiceMetrics::sink())
///     }
/// }
/// ```
///
/// # Generated Types
///
/// For a struct named `MyMetrics`, the macro generates:
/// - `MyMetricsEntry`: The internal representation used for serialization
/// - `MyMetricsGuard`: A wrapper that implements `Deref`/`DerefMut` to the original struct and handles emission on drop
/// - `MyMetricsHandle`: A shareable handle for concurrent access to the metrics
#[proc_macro_attribute]
pub fn metrics(attr: TokenStream, input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // There's a little bit of juggling here so we can return errors both from the root attribute & the inner attribute.
    // We will also write the compiler error from the root attribute into the token stream if it failed. But if it did fail,
    // we still analyze the main macro by passing in an empty root attributes instead.

    let mut base_token_stream = Ts2::new();
    let root_attrs = match parse_root_attrs(attr) {
        Ok(root_attrs) => root_attrs,
        Err(e) => {
            // recover and use an empty root attributes
            e.to_compile_error().to_tokens(&mut base_token_stream);
            RootAttributes::default()
        }
    };

    // Try to generate the full metrics implementation
    match generate_metrics(root_attrs, input.clone()) {
        Ok(output) => output.to_tokens(&mut base_token_stream),
        Err(err) => {
            // Always generate the base struct without metrics attributes to avoid cascading errors
            clean_base_struct(&input).to_tokens(&mut base_token_stream);
            // Include the error and the base struct without metrics attributes
            err.to_compile_error().to_tokens(&mut base_token_stream);
        }
    };
    base_token_stream.into()
}

#[derive(Debug, Default, FromMeta)]
struct RootAttributes {
    prefix: Option<String>,

    #[darling(default)]
    rename_all: NameStyle,

    #[darling(rename = "emf::dimension_sets")]
    emf_dimensions: Option<DimensionSets>,

    subfield: Flag,
}

impl RootAttributes {
    fn configuration_field_names(&self) -> Vec<Ts2> {
        if let Some(_dims) = &self.emf_dimensions {
            vec![quote! { __config__ }]
        } else {
            vec![]
        }
    }

    fn configuration_fields(&self) -> Vec<Ts2> {
        let mut fields = vec![];
        if let Some(_dims) = &self.emf_dimensions {
            fields.push(quote! {
                __config__: ::metrique::emf::SetEntryDimensions
            })
        }
        fields
    }

    fn create_configuration(&self) -> Vec<Ts2> {
        let mut fields = vec![];
        if let Some(dims) = &self.emf_dimensions {
            fields.push(quote! { __config__: ::metrique::__plumbing_entry_dimensions!(dims: #dims) })
        }
        fields
    }
}

#[derive(Debug, FromField)]
#[darling(attributes(metrics))]
struct RawMetricsFieldAttrs {
    flatten: Flag,

    flatten_entry: Flag,

    timestamp: Flag,

    ignore: Flag,

    #[darling(default)]
    unit: Option<SpannedKv<syn::Path>>,

    #[darling(default)]
    format: Option<SpannedKv<syn::Path>>,

    #[darling(default)]
    name: Option<SpannedKv<String>>,
}

/// Wrapper type to allow recovering both the key and value span when parsing an attribute
#[derive(Debug)]
struct SpannedKv<T> {
    key_span: Span,
    #[allow(dead_code)]
    value_span: Span,
    value: T,
}

impl<T: FromMeta> FromMeta for SpannedKv<T> {
    fn from_meta(item: &syn::Meta) -> darling::Result<Self> {
        let value = T::from_meta(item).map_err(|e| e.with_span(item))?;
        let (key_span, value_span) = match item {
            syn::Meta::NameValue(nv) => (nv.path.span(), nv.value.span()),
            _ => return Err(darling::Error::custom("expected a key value pair").with_span(item)),
        };

        Ok(SpannedKv {
            key_span,
            value_span,
            value,
        })
    }
}

// Set metrics to `new`, enforcing the fact that this field is exclusive and cannot be combined
fn set_exclusive(
    new: impl Fn(Span) -> MetricsFieldAttrs,
    name: &'static str,
    existing: Option<(MetricsFieldAttrs, &'static str)>,
    flag: &Flag,
) -> darling::Result<Option<(MetricsFieldAttrs, &'static str)>> {
    match (flag.is_present(), &existing) {
        (true, Some((_, other))) => Err(darling::Error::custom(format!(
            "Cannot combine {other} with {name}"
        ))
        .with_span(&flag.span())),
        (true, None) => Ok(Some((new(flag.span()), name))),
        _ => Ok(existing),
    }
}

// retrieve the value for a field, enforcing the fact that unit/name cannot be combined with other options
fn get_field_option<'a, T>(
    field_name: &'static str,
    existing: &Option<(MetricsFieldAttrs, &'static str)>,
    span: &'a Option<SpannedKv<T>>,
) -> darling::Result<Option<&'a T>> {
    match (span, &existing) {
        (Some(input), Some((_, other))) => Err(darling::Error::custom(format!(
            "Cannot combine {other} with {field_name}"
        ))
        .with_span(&input.key_span)),
        (Some(v), None) => Ok(Some(&v.value)),
        _ => Ok(None),
    }
}

impl RawMetricsFieldAttrs {
    fn validate(self) -> darling::Result<MetricsFieldAttrs> {
        let mut out: Option<(MetricsFieldAttrs, &'static str)> = None;
        out = set_exclusive(MetricsFieldAttrs::Flatten, "flatten", out, &self.flatten)?;
        out = set_exclusive(
            MetricsFieldAttrs::FlattenEntry,
            "flatten_entry",
            out,
            &self.flatten_entry,
        )?;
        out = set_exclusive(
            MetricsFieldAttrs::Timestamp,
            "timestamp",
            out,
            &self.timestamp,
        )?;
        out = set_exclusive(MetricsFieldAttrs::Ignore, "ignore", out, &self.ignore)?;

        let name = self.name.map(validate_name).transpose()?;
        let name = get_field_option("name", &out, &name)?;
        let unit = get_field_option("unit", &out, &self.unit)?;
        let format = get_field_option("format", &out, &self.format)?;
        Ok(match out {
            Some((out, _)) => out,
            None => MetricsFieldAttrs::Field {
                name: name.cloned(),
                unit: unit.cloned(),
                format: format.cloned(),
            },
        })
    }
}

fn validate_name(name: SpannedKv<String>) -> darling::Result<SpannedKv<String>> {
    match validate_name_inner(&name.value) {
        Ok(_) => Ok(name),
        Err(msg) => Err(darling::Error::custom(msg).with_span(&name.value_span)),
    }
}

fn validate_name_inner(name: &str) -> std::result::Result<(), &'static str> {
    if name.is_empty() {
        return Err("invalid name: name field must not be empty");
    }

    if name.contains(' ') {
        return Err("invalid name: name must not contain spaces");
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum MetricsFieldAttrs {
    Ignore(Span),
    Flatten(Span),
    FlattenEntry(Span),
    Timestamp(Span),
    Field {
        unit: Option<syn::Path>,
        name: Option<String>,
        format: Option<syn::Path>,
    },
}

fn parse_root_attrs(attr: TokenStream) -> Result<RootAttributes> {
    let nested_meta = NestedMeta::parse_meta_list(attr.into())?;
    Ok(RootAttributes::from_list(&nested_meta)?)
}

fn generate_metrics(root_attributes: RootAttributes, input: DeriveInput) -> Result<Ts2> {
    // Extract the struct name and create derived names
    let struct_name = &input.ident;
    let entry_name = format_ident!("{}Entry", struct_name);
    let guard_name = format_ident!("{}Guard", struct_name);
    let handle_name = format_ident!("{}Handle", struct_name);

    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => &fields_named.named,
            _ => {
                return Err(Error::new_spanned(
                    &input,
                    "Only named fields are supported",
                ));
            }
        },
        _ => return Err(Error::new_spanned(&input, "Only structs are supported")),
    };

    let parsed_fields = parse_struct_fields(fields)?;

    let base_struct = generate_base_struct(
        struct_name,
        &input.vis,
        &input.generics,
        &input.attrs,
        &parsed_fields,
    )?;

    // No longer need to derive Entry since we're implementing it directly in entry_impl.rs
    let entry_struct = generate_entry_struct(
        &entry_name,
        &input.generics,
        &parsed_fields,
        &root_attributes,
    )?;

    // Generate the Entry trait implementation
    let entry_impl = entry_impl::generate_entry_impl(&entry_name, &parsed_fields, &root_attributes);

    let close_value_impl =
        generate_close_value_impl(struct_name, &entry_name, &parsed_fields, &root_attributes);
    let vis = &input.vis;

    let root_entry_specifics = if root_attributes.subfield.is_present() {
        quote! {}
    } else {
        // Generate the on_drop_wrapper implementation
        let on_drop_wrapper =
            generate_on_drop_wrapper(vis, &guard_name, struct_name, &entry_name, &handle_name);
        quote! {
            // the <STRUCT>Guard that implements AppendOnDrop
            #on_drop_wrapper
        }
    };

    // Generate the final output
    let output = quote! {
        // The struct provided to the proc macro, minus the #[metrics] attrs
        #base_struct

        // The struct that implements the entry trait
        #entry_struct

        // The Entry trait implementation
        #entry_impl

        // the implementation of CloseValue for base_struct
        #close_value_impl

        #root_entry_specifics
    };

    if std::env::var("MACRO_DEBUG").is_ok() {
        eprintln!("{}", &output);
    }

    Ok(output)
}

fn generate_base_struct(
    name: &Ident,
    vis: &Visibility,
    generics: &Generics,
    attrs: &[Attribute],
    fields: &[MetricsField],
) -> Result<Ts2> {
    let fields = fields.iter().map(|f| f.core_field());
    let data = quote! {
        #(#fields),*
    };
    let expanded = quote! {
        #(#attrs)*
        #vis struct #name #generics { #data }
    };

    Ok(expanded)
}

/// Generate the on_drop_wrapper implementation
fn generate_on_drop_wrapper(
    vis: &Visibility,
    guard: &Ident,
    inner: &Ident,
    target: &Ident,
    handle: &Ident,
) -> Ts2 {
    quote! {
        #vis type #guard<Q = ::metrique::DefaultSink> = ::metrique::AppendAndCloseOnDrop<#inner, Q>;
        #vis type #handle<Q = ::metrique::DefaultSink> = ::metrique::AppendAndCloseOnDropHandle<#inner, Q>;

        impl #inner {
            #[doc = "Creates a AppendAndCloseOnDrop that will be automatically appended to `sink` on drop."]
            #vis fn append_on_drop<Q: ::metrique::__writer::EntrySink<::metrique::RootEntry<#target>> + Send + Sync + 'static>(self, sink: Q) -> #guard<Q> {
                ::metrique::append_and_close(self, sink)
            }
        }
    }
}

fn generate_close_value_impl(
    metrics_struct: &Ident,
    entry: &Ident,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let fields = fields
        .iter()
        .filter(|f| !matches!(f.attrs, MetricsFieldAttrs::Ignore(_)))
        .map(|f| f.close_value());
    let config = root_attrs.create_configuration();
    quote! {
        impl metrique::CloseValue for #metrics_struct {
            type Closed = #entry;
            fn close(self) -> Self::Closed {
                #[allow(deprecated)]
                #entry {
                    #(#config,)*
                    #(#fields,)*
                }
            }
        }
    }
}

fn generate_entry_struct(
    name: &Ident,
    _generics: &Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Result<Ts2> {
    let fields = fields.iter().flat_map(|f| f.entry_field());
    let config = root_attrs.configuration_fields();
    let data = quote! {
        #(#config,)*
        #(#fields,)*
    };
    let expanded = quote! {
        #[doc(hidden)]
        pub struct #name {
            #data
        }
    };

    Ok(expanded)
}

/// Parse the fields of a struct into a vector of MField objects
fn parse_struct_fields(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> Result<Vec<MetricsField>> {
    let mut parsed_fields = vec![];
    let mut errors = darling::Error::accumulator();

    // Process each field
    for field in fields {
        let field_name = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new_spanned(field, "Field must have a name"))?;

        // Parse field attributes using darling
        let attrs = match errors
            .handle(RawMetricsFieldAttrs::from_field(field).and_then(|attr| attr.validate()))
        {
            Some(attrs) => attrs,
            None => {
                continue;
            }
        };

        let mut external_attrs = vec![];
        for attr in &field.attrs {
            if !attr.path().is_ident("metrics") {
                external_attrs.push(attr.clone());
            }
        }

        parsed_fields.push(MetricsField {
            ident: field_name.clone(),
            ty: field.ty.clone(),
            vis: field.vis.clone(),
            external_attrs,
            attrs,
        });
    }

    errors.finish()?;

    Ok(parsed_fields)
}

struct MetricsField {
    vis: Visibility,
    ident: Ident,
    ty: Type,
    external_attrs: Vec<Attribute>,
    attrs: MetricsFieldAttrs,
}

impl MetricsField {
    fn core_field(&self) -> Ts2 {
        let MetricsField {
            ref external_attrs,
            ref ident,
            ref ty,
            ref vis,
            ..
        } = *self;
        quote! { #(#external_attrs)* #vis #ident: #ty }
    }

    fn entry_field(&self) -> Option<Ts2> {
        let ident_span = self.ident.span();
        if let MetricsFieldAttrs::Ignore(_span) = self.attrs {
            return None;
        }
        let &MetricsField { ident, ty, .. } = &self;
        let mut base_type = quote_spanned! { ident_span=>
            <#ty as metrique::CloseValue>::Closed
        };
        if let MetricsFieldAttrs::FlattenEntry(_) = self.attrs {
            base_type = quote_spanned! { ident_span=>#ty };
        }
        if let Some(expr) = self.unit() {
            base_type = quote_spanned! { expr.span()=>
                <#base_type as ::metrique::unit::AttachUnit>::Output<#expr>
            }
        }
        Some(quote_spanned! { ident_span=>
                #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique_writer::test_util::test_entry`")]
                #[doc(hidden)]
                #ident: #base_type
        })
    }

    fn unit(&self) -> Option<&syn::Path> {
        match &self.attrs {
            MetricsFieldAttrs::Field { unit, .. } => unit.as_ref(),
            _ => None,
        }
    }

    fn close_value(&self) -> Ts2 {
        let ident = &self.ident;
        let base = if let MetricsFieldAttrs::FlattenEntry(_) = self.attrs {
            quote_spanned! { ident.span() => self.#ident }
        } else {
            let mut base = quote_spanned! {
                ident.span() => metrique::CloseValue::close(self.#ident)
            };

            if let Some(unit) = self.unit() {
                base = quote_spanned! { unit.span() =>
                    #base.into()
                }
            }

            base
        };

        quote! { #ident: #base }
    }
}

/// Minimal passthrough that strips #[metrics] attributes from struct fields.
///
/// If the proc macro fails, then absent anything else, the struct provider by the user will
/// not exist in code. This ensures that even if the proc macro errors, the struct will still be present
/// making finding the actual cause of the compiler errors much easier.
///
/// This function is not used in the happy path case, but if we encounter errors in the
/// main pass, this is returned along with the compiler error to remove spurious compiler
/// failures.
fn clean_base_struct(input: &DeriveInput) -> Ts2 {
    let struct_name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;

    // Filter out any #[metrics] attributes from the struct
    let filtered_attrs: Vec<_> = input
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("metrics"))
        .collect();

    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => &fields_named.named,
            // In these cases, we can't strip attributes since we don't support this format.
            // Echo back exactly what was given.
            _ => return input.to_token_stream(),
        },
        _ => return input.to_token_stream(),
    };

    // Strip out `metrics` attribute
    let clean_fields = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let field_vis = &field.vis;

        // Filter out metrics attributes
        let field_attrs: Vec<_> = field
            .attrs
            .iter()
            .filter(|attr| !attr.path().is_ident("metrics"))
            .collect();

        quote! {
            #(#field_attrs)*
            #field_vis #field_name: #field_type
        }
    });

    let expanded = quote! {
        #(#filtered_attrs)*
        #vis struct #struct_name #generics {
            #(#clean_fields),*
        }
    };

    expanded
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use proc_macro2::TokenStream as Ts2;
    use quote::quote;
    use syn::{parse_quote, parse2};

    use crate::RootAttributes;

    // Helper function to convert proc_macro::TokenStream to proc_macro2::TokenStream
    // This allows us to test the macro without needing to use the proc_macro API directly
    fn metrics_impl(input: Ts2) -> Ts2 {
        let input = syn::parse2(input).unwrap();
        let root_attrs = RootAttributes::default();
        super::generate_metrics(root_attrs, input).unwrap()
    }

    #[test]
    fn test_darling_root_attrs() {
        use darling::FromMeta;
        RootAttributes::from_meta(&parse_quote! {
            metrics(
                rename_all = "PascalCase",
                emf::dimension_sets = [["bar"]]
            )
        })
        .unwrap();
    }

    #[test]
    fn test_simple_metrics_struct() {
        let input = quote! {
            struct RequestMetrics {
                operation: &'static str,
                number_of_ducks: usize
            }
        };

        // Process the input through the metrics macro
        let output = metrics_impl(input);

        // Parse the output back into a syn::File for pretty printing
        let parsed_file = match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => {
                // If parsing fails, use the raw string output
                output.to_string()
            }
        };

        assert_snapshot!("simple_metrics_struct", parsed_file);
    }
}
