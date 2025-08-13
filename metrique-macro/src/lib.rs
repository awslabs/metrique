// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

mod emf;
mod entry_impl;
mod inflect;
mod value_impl;

use darling::{FromField, FromMeta, FromVariant, ast::NestedMeta, util::Flag};
use emf::DimensionSets;
use inflect::NameStyle;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Ts2};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, FieldsNamed, FieldsUnnamed, Generics, Ident,
    Result, Type, Visibility, parse_macro_input, spanned::Spanned,
};

use crate::inflect::metric_name;

/// Transforms a struct or enum into a unit-of-work metric.
///
/// Currently, enums are only supported with `value(string)`.
///
/// # Container Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `rename_all` | String | Changes the case style of all field names | `#[metrics(rename_all = "PascalCase")]` |
/// | `prefix` | String | Adds a prefix to all field names | `#[metrics(prefix = "api_")]` |
/// | `emf::dimension_sets` | Array | Defines dimension sets for CloudWatch metrics | `#[metrics(emf::dimension_sets = [["Status", "Operation"]])]` |
/// | `subfield` | Flag | When set, this metric can only be used when nested within other metrics, and can be consumed by reference (has both `impl CloseValue for &MyStruct` and `impl CloseValue for MyStruct`). It cannot be added to a sink directly. | `#[metrics(subfield)]` |
/// | `subfield_owned` | Flag | When set, this metric can only be used when nested within other metrics. It cannot be added to a sink directly. | `#[metrics(subfield_owned)]` |
/// | `value` | Flag | Used for *structs*. Makes the struct a value newtype | `#[metrics(value)]` |
/// | `value(string)` | Flag | Used for *enums*. Transforms the enum into a string value. | `#[metrics(value(string))]` |
///
/// # Field Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `name` | String | Overrides the field name in metrics | `#[metrics(name = "CustomName")]` |
/// | `unit` | Path | Specifies the unit for the metric value | `#[metrics(unit = Millisecond)]` |
/// | `timestamp` | Flag | Marks a field as the canonical timestamp | `#[metrics(timestamp)]` |
/// | `prefix` | Flag | Adds a prefix to flattened entries. Prefix will get inflected to the right case style | `#[metrics(flatten, prefix="prefix-")]` |
/// | `flatten` | Flag | Flattens nested `CloseEntry` metric structs | `#[metrics(flatten)]` |
/// | `flatten_entry` | Flag | Flattens nested `CloseValue<Closed: Entry>` metric structs | `#[metrics(flatten_entry)]` |
/// | `no_close` | Flag | Use the entry directly instead of closing it | `#[metrics(no_close)]` |
/// | `ignore` | Flag | Excludes the field from metrics | `#[metrics(ignore)]` |
///
/// # Variant Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `name` | String | Overrides the field name in metrics | `#[metrics(name = "CustomName")]` |
///
/// # Example
///
/// ```rust,ignore
/// use metrique::unit_of_work::metrics;
/// use metrique::timers::{Timestamp, Timer};
/// use metrique::unit::{Count, Millisecond};
/// use metrique_writer::{GlobalEntrySink, ServiceMetrics};
/// use std::time::SystemTime;
///
/// #[metrics(value(string), rename_all = "snake_case")]
/// enum Operation {
///    CountDucks
/// }
///
/// #[metrics(value)]
/// struct RequestCount(#[metrics(unit=Count)] usize);
///
/// #[metrics(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     operation: Operation,
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
///     request_count: RequestCount,
/// }
///
/// #[metrics(subfield, prefix = "sub_")]
/// struct NestedMetrics {
///     #[metrics(name = "CustomCounter")]
///     counter: usize,
/// }
///
/// impl RequestMetrics {
///     fn init(operation: Operation) -> RequestMetricsGuard {
///         RequestMetrics {
///             timestamp: SystemTime::now(),
///             operation,
///             operation_time: Timer::start_now(),
///             nested: NestedMetrics { counter: 0 },
///             request_count: RequestCount(0),
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
            clean_base_adt(&input).to_tokens(&mut base_token_stream);
            // Include the error and the base struct without metrics attributes
            err.to_compile_error().to_tokens(&mut base_token_stream);
        }
    };
    base_token_stream.into()
}

#[derive(Copy, Clone, Debug)]
enum OwnershipKind {
    ByRef,
    ByValue,
}

#[derive(Debug, Default, FromMeta)]
// allow both `#[metric(value)]` and `#[metric(value(string))]` to be parsed
#[darling(from_word = Self::from_word)]
struct ValueAttributes {
    string: Flag,
}

impl ValueAttributes {
    /// constructor used in case of the `#[metric(value)]` form
    fn from_word() -> darling::Result<Self> {
        Ok(Self::default())
    }
}

#[derive(Debug, Default, FromMeta)]
struct RawRootAttributes {
    prefix: Option<String>,

    #[darling(default)]
    rename_all: NameStyle,

    #[darling(rename = "emf::dimension_sets")]
    emf_dimensions: Option<DimensionSets>,

    subfield: Flag,
    #[darling(rename = "subfield_owned")]
    subfield_owned: Flag,
    value: Option<ValueAttributes>,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
enum MetricMode {
    #[default]
    RootEntry,
    Subfield,
    SubfieldOwned,
    Value,
    ValueString,
}

#[derive(Debug, Default)]
struct RootAttributes {
    prefix: Option<String>,

    rename_all: NameStyle,

    emf_dimensions: Option<DimensionSets>,

    mode: MetricMode,
}

impl RawRootAttributes {
    fn validate(self) -> darling::Result<RootAttributes> {
        let mut out: Option<(MetricMode, &'static str)> = None;
        if let Some(value_attrs) = self.value {
            if value_attrs.string.is_present() {
                out = set_exclusive(
                    |_| MetricMode::ValueString,
                    "value",
                    out,
                    &value_attrs.string,
                )?
            } else {
                out = Some((MetricMode::Value, "value"));
            }
        }
        out = set_exclusive(|_| MetricMode::Subfield, "subfield", out, &self.subfield)?;
        out = set_exclusive(
            |_| MetricMode::SubfieldOwned,
            "subfield_owned",
            out,
            &self.subfield_owned,
        )?;
        let mode = out.map(|(s, _)| s).unwrap_or_default();
        if let (MetricMode::ValueString, Some(ds)) = (mode, &self.emf_dimensions) {
            return Err(
                darling::Error::custom("value does not make sense with dimension-sets")
                    .with_span(&ds.span()),
            );
        }
        Ok(RootAttributes {
            prefix: self.prefix,
            rename_all: self.rename_all,
            emf_dimensions: self.emf_dimensions,
            mode,
        })
    }
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
            fields
                .push(quote! { __config__: ::metrique::__plumbing_entry_dimensions!(dims: #dims) })
        }
        fields
    }

    fn ownership_kind(&self) -> OwnershipKind {
        match self.mode {
            MetricMode::RootEntry | MetricMode::SubfieldOwned => OwnershipKind::ByValue,
            MetricMode::Subfield | MetricMode::ValueString | MetricMode::Value => {
                OwnershipKind::ByRef
            }
        }
    }
}

#[derive(Debug, FromField)]
#[darling(attributes(metrics))]
struct RawMetricsFieldAttrs {
    flatten: Flag,

    flatten_entry: Flag,

    no_close: Flag,

    timestamp: Flag,

    ignore: Flag,

    #[darling(default)]
    unit: Option<SpannedKv<syn::Path>>,

    #[darling(default)]
    format: Option<SpannedKv<syn::Path>>,

    #[darling(default)]
    name: Option<SpannedKv<String>>,

    #[darling(default)]
    prefix: Option<SpannedKv<String>>,
}

#[derive(Debug, FromVariant)]
#[darling(attributes(metrics))]
struct RawMetricsVariantAttrs {
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
fn set_exclusive<T>(
    new: impl Fn(Span) -> T,
    name: &'static str,
    existing: Option<(T, &'static str)>,
    flag: &Flag,
) -> darling::Result<Option<(T, &'static str)>> {
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
    existing: &Option<(MetricsFieldKind, &'static str)>,
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

impl RawMetricsVariantAttrs {
    fn validate(self) -> darling::Result<MetricsVariantAttrs> {
        Ok(MetricsVariantAttrs {
            name: self.name.map(|n| n.value),
        })
    }
}

impl RawMetricsFieldAttrs {
    fn validate(self) -> darling::Result<MetricsFieldAttrs> {
        let mut out: Option<(MetricsFieldKind, &'static str)> = None;
        out = set_exclusive(
            |span| MetricsFieldKind::Flatten { span, prefix: None },
            "flatten",
            out,
            &self.flatten,
        )?;
        out = set_exclusive(
            MetricsFieldKind::FlattenEntry,
            "flatten_entry",
            out,
            &self.flatten_entry,
        )?;
        out = set_exclusive(
            MetricsFieldKind::Timestamp,
            "timestamp",
            out,
            &self.timestamp,
        )?;
        out = set_exclusive(MetricsFieldKind::Ignore, "ignore", out, &self.ignore)?;

        let name = self.name.map(validate_name).transpose()?;
        let name = get_field_option("name", &out, &name)?;
        let unit = get_field_option("unit", &out, &self.unit)?;
        let format = get_field_option("format", &out, &self.format)?;
        let close = !self.no_close.is_present();
        if let (false, Some((MetricsFieldKind::Ignore(span), _))) = (close, &out) {
            return Err(
                darling::Error::custom("Cannot combine ignore with no_close").with_span(span),
            );
        }
        if let Some(prefix_val) = self.prefix {
            match &mut out {
                Some((MetricsFieldKind::Flatten { span: _, prefix }, _)) => {
                    *prefix = Some(prefix_val.value);
                }
                _ => {
                    return Err(
                        darling::Error::custom("prefix can only be used with flatten")
                            .with_span(&prefix_val.key_span),
                    );
                }
            }
        }
        Ok(MetricsFieldAttrs {
            close,
            kind: match out {
                Some((out, _)) => out,
                None => MetricsFieldKind::Field {
                    name: name.cloned(),
                    unit: unit.cloned(),
                    format: format.cloned(),
                },
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

#[derive(Debug, Default, Clone)]
struct MetricsVariantAttrs {
    name: Option<String>,
}

#[derive(Debug, Clone)]
struct MetricsFieldAttrs {
    close: bool,
    kind: MetricsFieldKind,
}

#[derive(Debug, Clone)]
enum MetricsFieldKind {
    Ignore(Span),
    Flatten {
        span: Span,
        prefix: Option<String>,
    },
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
    Ok(RawRootAttributes::from_list(&nested_meta)?.validate()?)
}

fn generate_metrics(root_attributes: RootAttributes, input: DeriveInput) -> Result<Ts2> {
    let output = match root_attributes.mode {
        MetricMode::RootEntry
        | MetricMode::Subfield
        | MetricMode::SubfieldOwned
        | MetricMode::Value => {
            let fields = match &input.data {
                Data::Struct(data_struct) => match &data_struct.fields {
                    Fields::Named(fields_named) => &fields_named.named,
                    Fields::Unnamed(fields_unnamed)
                        if root_attributes.mode == MetricMode::Value =>
                    {
                        &fields_unnamed.unnamed
                    }
                    _ => {
                        return Err(Error::new_spanned(
                            &input,
                            "Only named fields are supported",
                        ));
                    }
                },
                _ => {
                    return Err(Error::new_spanned(
                        &input,
                        "Only structs are supported for entries",
                    ));
                }
            };
            generate_metrics_for_struct(root_attributes, &input, fields)?
        }
        MetricMode::ValueString => {
            let variants = match &input.data {
                Data::Enum(data_enum) => &data_enum.variants,
                _ => {
                    return Err(Error::new_spanned(
                        &input,
                        "Only enums are supported for values",
                    ));
                }
            };
            generate_metrics_for_enum(root_attributes, &input, variants)?
        }
    };

    if std::env::var("MACRO_DEBUG").is_ok() {
        eprintln!("{}", &output);
    }

    Ok(output)
}

fn generate_metrics_for_enum(
    root_attrs: RootAttributes,
    input: &DeriveInput,
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
) -> Result<Ts2> {
    let enum_name = &input.ident;
    let parsed_variants = parse_enum_variants(variants, true)?;
    let value_name = format_ident!("{}Value", enum_name);

    let base_enum = generate_base_enum(
        enum_name,
        &input.vis,
        &input.generics,
        &input.attrs,
        &parsed_variants,
    );

    let value_enum =
        generate_value_enum(&value_name, &input.generics, &parsed_variants, &root_attrs)?;

    let value_impl =
        value_impl::generate_value_impl_for_enum(&root_attrs, &value_name, &parsed_variants);

    let variants_map = parsed_variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        quote_spanned!(variant.ident.span()=> #enum_name::#variant_ident => #value_name::#variant_ident)
    });
    let variants_map = quote!(#[allow(deprecated)] match self { #(#variants_map),* });

    let close_value_impl =
        generate_close_value_impls(&root_attrs, enum_name, &value_name, variants_map);

    Ok(quote! {
        #base_enum
        #value_enum
        #value_impl
        #close_value_impl
    })
}

fn generate_metrics_for_struct(
    root_attributes: RootAttributes,
    input: &DeriveInput,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> Result<Ts2> {
    // Extract the struct name and create derived names
    let struct_name = &input.ident;
    let entry_name = if root_attributes.mode == MetricMode::Value {
        format_ident!("{}Value", struct_name)
    } else {
        format_ident!("{}Entry", struct_name)
    };
    let guard_name = format_ident!("{}Guard", struct_name);
    let handle_name = format_ident!("{}Handle", struct_name);

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
    let inner_impl = match root_attributes.mode {
        MetricMode::Value => {
            value_impl::validate_value_impl_for_struct(
                &root_attributes,
                &entry_name,
                &parsed_fields,
            )?;
            value_impl::generate_value_impl_for_struct(
                &root_attributes,
                &entry_name,
                &parsed_fields,
            )?
        }
        _ => entry_impl::generate_entry_impl(&entry_name, &parsed_fields, &root_attributes),
    };

    let close_value_impl = generate_close_value_impls_for_struct(
        struct_name,
        &entry_name,
        &parsed_fields,
        &root_attributes,
    );
    let vis = &input.vis;

    let root_entry_specifics = match root_attributes.mode {
        MetricMode::RootEntry => {
            // Generate the on_drop_wrapper implementation
            let on_drop_wrapper =
                generate_on_drop_wrapper(vis, &guard_name, struct_name, &entry_name, &handle_name);
            quote! {
                // the <STRUCT>Guard that implements AppendOnDrop
                #on_drop_wrapper
            }
        }
        MetricMode::Subfield
        | MetricMode::SubfieldOwned
        | MetricMode::ValueString
        | MetricMode::Value => {
            quote! {}
        }
    };

    // Generate the final output
    Ok(quote! {
        // The struct provided to the proc macro, minus the #[metrics] attrs
        #base_struct

        // The struct that implements the entry trait
        #entry_struct

        // The Entry trait implementation
        #inner_impl

        // the implementation of CloseValue for base_struct
        #close_value_impl

        #root_entry_specifics
    })
}

fn generate_base_struct(
    name: &Ident,
    vis: &Visibility,
    generics: &Generics,
    attrs: &[Attribute],
    fields: &[MetricsField],
) -> Result<Ts2> {
    let has_named_fields = fields.iter().any(|f| f.name.is_some());
    let fields = fields.iter().map(|f| f.core_field(has_named_fields));
    let body = wrap_fields_into_struct_decl(has_named_fields, fields);

    Ok(quote! {
        #(#attrs)*
        #vis struct #name #generics #body
    })
}

fn generate_base_enum(
    name: &Ident,
    vis: &Visibility,
    generics: &Generics,
    attrs: &[Attribute],
    variants: &[MetricsVariant],
) -> Ts2 {
    let variants = variants.iter().map(|f| f.core_variant());
    let data = quote! {
        #(#variants),*
    };
    let expanded = quote! {
        #(#attrs)*
        #vis enum #name #generics { #data }
    };

    expanded
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

fn generate_close_value_impls(
    root_attrs: &RootAttributes,
    base_ty: &Ident,
    closed_ty: &Ident,
    impl_body: Ts2,
) -> Ts2 {
    let (metrics_struct_ty, proxy_impl) = match root_attrs.ownership_kind() {
        OwnershipKind::ByValue => (quote!(#base_ty), quote!()),
        OwnershipKind::ByRef => (
            quote!(&'_ #base_ty),
            // for a by-ref ownership, also add a proxy impl for by-value
            quote!(impl metrique::CloseValue for #base_ty {
                type Closed = #closed_ty;
                fn close(self) -> Self::Closed {
                    <&Self>::close(&self)
                }
            }),
        ),
    };
    quote! {
        impl metrique::CloseValue for #metrics_struct_ty {
            type Closed = #closed_ty;
            fn close(self) -> Self::Closed {
                #impl_body
            }
        }

        #proxy_impl
    }
}

fn generate_close_value_impls_for_struct(
    metrics_struct: &Ident,
    entry: &Ident,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Ts2 {
    let fields = fields
        .iter()
        .filter(|f| !matches!(f.attrs.kind, MetricsFieldKind::Ignore(_)))
        .map(|f| f.close_value(root_attrs.ownership_kind()));
    let config: Vec<Ts2> = root_attrs.create_configuration();
    generate_close_value_impls(
        root_attrs,
        metrics_struct,
        entry,
        quote! {
            #[allow(deprecated)]
            #entry {
                #(#config,)*
                #(#fields,)*
            }
        },
    )
}

fn wrap_fields_into_struct_decl(
    has_named_fields: bool,
    data: impl IntoIterator<Item = Ts2>,
) -> Ts2 {
    let data = data.into_iter();
    if has_named_fields {
        quote! { { #(#data),* } }
    } else {
        quote! { ( #(#data),* ); }
    }
}

fn generate_entry_struct(
    name: &Ident,
    _generics: &Generics,
    fields: &[MetricsField],
    root_attrs: &RootAttributes,
) -> Result<Ts2> {
    let has_named_fields = fields.iter().any(|f| f.name.is_some());
    let config = root_attrs.configuration_fields();

    let fields = fields.iter().flat_map(|f| f.entry_field(has_named_fields));
    let body = wrap_fields_into_struct_decl(has_named_fields, config.into_iter().chain(fields));
    Ok(quote!(
        #[doc(hidden)]
        pub struct #name #body
    ))
}

fn generate_value_enum(
    name: &Ident,
    _generics: &Generics,
    variants: &[MetricsVariant],
    _root_attrs: &RootAttributes,
) -> Result<Ts2> {
    let variants = variants.iter().map(|variant| variant.entry_variant());
    let data = quote! {
        #(#variants,)*
    };
    let expanded = quote! {
        #[doc(hidden)]
        pub enum #name {
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
    for (i, field) in fields.iter().enumerate() {
        let i = syn::Index::from(i);
        let (ident, name, span) = match &field.ident {
            Some(ident) => (quote! { #ident }, Some(ident.to_string()), ident.span()),
            None => (quote! { #i }, None, field.ty.span()),
        };
        // Parse field attributes using darling
        let attrs = match errors
            .handle(RawMetricsFieldAttrs::from_field(field).and_then(|attr| attr.validate()))
        {
            Some(attrs) => attrs,
            None => {
                continue;
            }
        };

        parsed_fields.push(MetricsField {
            ident,
            name,
            span,
            ty: field.ty.clone(),
            vis: field.vis.clone(),
            external_attrs: clean_attrs(&field.attrs),
            attrs,
        });
    }

    errors.finish()?;

    Ok(parsed_fields)
}

/// Parse the variants of an enum into a vector of MField objects
fn parse_enum_variants(
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
    parse_attrs: bool,
) -> Result<Vec<MetricsVariant>> {
    let mut parsed_variants = vec![];
    let mut errors = darling::Error::accumulator();

    // Process each field
    for variant in variants {
        if !variant.fields.is_empty() {
            return Err(Error::new_spanned(
                variant,
                "variants with fields are not supported",
            ));
        }

        let attrs = if parse_attrs {
            // Currently there are no variant attributes
            match errors.handle(RawMetricsVariantAttrs::from_variant(variant)) {
                Some(attrs) => attrs.validate()?,
                None => {
                    continue;
                }
            }
        } else {
            MetricsVariantAttrs::default()
        };

        parsed_variants.push(MetricsVariant {
            ident: variant.ident.clone(),
            external_attrs: clean_attrs(&variant.attrs),
            attrs,
        });
    }

    errors.finish()?;

    Ok(parsed_variants)
}

struct MetricsVariant {
    ident: Ident,
    external_attrs: Vec<Attribute>,
    attrs: MetricsVariantAttrs,
}

impl MetricsVariant {
    fn core_variant(&self) -> Ts2 {
        let MetricsVariant {
            ref external_attrs,
            ref ident,
            ..
        } = *self;
        quote! { #(#external_attrs)* #ident }
    }

    fn entry_variant(&self) -> Ts2 {
        let ident_span = self.ident.span();
        let ident = &self.ident;
        quote_spanned! { ident_span=>
            #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique_writer::test_util::test_entry`")]
            #[doc(hidden)]
            #ident
        }
    }
}

struct MetricsField {
    vis: Visibility,
    ident: Ts2,
    name: Option<String>,
    span: Span,
    ty: Type,
    external_attrs: Vec<Attribute>,
    attrs: MetricsFieldAttrs,
}

impl MetricsField {
    fn core_field(&self, is_named: bool) -> Ts2 {
        let MetricsField {
            ref external_attrs,
            ref ident,
            ref ty,
            ref vis,
            ..
        } = *self;
        let field = if is_named {
            quote! { #ident: #ty }
        } else {
            quote! { #ty }
        };
        quote! { #(#external_attrs)* #vis #field }
    }

    fn entry_field(&self, named: bool) -> Option<Ts2> {
        if let MetricsFieldKind::Ignore(_span) = self.attrs.kind {
            return None;
        }
        let MetricsField {
            ident, ty, span, ..
        } = self;
        let mut base_type = if self.attrs.close {
            quote_spanned! { *span=> <#ty as metrique::CloseValue>::Closed }
        } else {
            quote_spanned! { *span=>#ty }
        };
        if let Some(expr) = self.unit() {
            base_type = quote_spanned! { expr.span()=>
                <#base_type as ::metrique::unit::AttachUnit>::Output<#expr>
            }
        }
        let inner = if named {
            quote! { #ident: #base_type }
        } else {
            quote! { #base_type }
        };
        Some(quote_spanned! { *span=>
                #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique_writer::test_util::test_entry`")]
                #[doc(hidden)]
                #inner
        })
    }

    fn unit(&self) -> Option<&syn::Path> {
        match &self.attrs.kind {
            MetricsFieldKind::Field { unit, .. } => unit.as_ref(),
            _ => None,
        }
    }

    fn close_value(&self, ownership_kind: OwnershipKind) -> Ts2 {
        let ident = &self.ident;
        let span = self.span;
        let field_expr = match ownership_kind {
            OwnershipKind::ByValue => quote_spanned! {span=> self.#ident },
            OwnershipKind::ByRef => quote_spanned! {span=> &self.#ident },
        };
        let base = if self.attrs.close {
            quote_spanned! {span=> metrique::CloseValue::close(#field_expr) }
        } else {
            field_expr
        };

        let base = if let Some(unit) = self.unit() {
            quote_spanned! { unit.span() =>
                #base.into()
            }
        } else {
            base
        };

        quote! { #ident: #base }
    }
}

fn clean_attrs(attr: &[Attribute]) -> Vec<Attribute> {
    attr.iter()
        .filter(|attr| !attr.path().is_ident("metrics"))
        .cloned()
        .collect()
}

fn clean_base_struct(
    vis: &syn::Visibility,
    struct_name: &syn::Ident,
    generics: &syn::Generics,
    filtered_attrs: Vec<Attribute>,
    fields: &FieldsNamed,
) -> Ts2 {
    // Strip out `metrics` attribute
    let clean_fields = fields.named.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let field_vis = &field.vis;

        // Filter out metrics attributes
        let field_attrs = clean_attrs(&field.attrs);

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

fn clean_base_unnamed_struct(
    vis: &syn::Visibility,
    struct_name: &syn::Ident,
    generics: &syn::Generics,
    filtered_attrs: Vec<Attribute>,
    fields: &FieldsUnnamed,
) -> Ts2 {
    // Strip out `metrics` attribute
    let clean_fields = fields.unnamed.iter().map(|field| {
        let field_type = &field.ty;
        let field_vis = &field.vis;

        // Filter out metrics attributes
        let field_attrs = clean_attrs(&field.attrs);

        quote! {
            #(#field_attrs)*
            #field_vis #field_type
        }
    });

    let expanded = quote! {
        #(#filtered_attrs)*
        #vis struct #struct_name #generics (
            #(#clean_fields),*
        );
    };

    expanded
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
fn clean_base_adt(input: &DeriveInput) -> Ts2 {
    let adt_name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;

    // Filter out any #[metrics] attributes from the struct
    let filtered_attrs = clean_attrs(&input.attrs);
    match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => {
                clean_base_struct(vis, adt_name, generics, filtered_attrs, fields_named)
            }
            Fields::Unnamed(fields_unnamed) => {
                clean_base_unnamed_struct(vis, adt_name, generics, filtered_attrs, fields_unnamed)
            }
            // In these cases, we can't strip attributes since we don't support this format.
            // Echo back exactly what was given.
            _ => input.to_token_stream(),
        },
        Data::Enum(data_enum) => {
            if let Ok(variants) = parse_enum_variants(&data_enum.variants, false) {
                generate_base_enum(adt_name, vis, generics, &filtered_attrs, &variants)
            } else {
                input.to_token_stream()
            }
        }
        _ => input.to_token_stream(),
    }
}

#[cfg(test)]
mod tests {
    use darling::FromMeta;
    use insta::assert_snapshot;
    use proc_macro2::TokenStream as Ts2;
    use quote::quote;
    use syn::{parse_quote, parse2};

    use crate::RawRootAttributes;

    // Helper function to convert proc_macro::TokenStream to proc_macro2::TokenStream
    // This allows us to test the macro without needing to use the proc_macro API directly
    fn metrics_impl(input: Ts2, attrs: Ts2) -> Ts2 {
        let input = syn::parse2(input).unwrap();
        let meta: syn::Meta = syn::parse2(attrs).unwrap();
        let root_attrs = RawRootAttributes::from_meta(&meta)
            .unwrap()
            .validate()
            .unwrap();
        super::generate_metrics(root_attrs, input).unwrap()
    }

    #[test]
    fn test_darling_root_attrs() {
        use darling::FromMeta;
        RawRootAttributes::from_meta(&parse_quote! {
            metrics(
                rename_all = "PascalCase",
                emf::dimension_sets = [["bar"]]
            )
        })
        .unwrap()
        .validate()
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
        let output = metrics_impl(input, quote!(metrics()));

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

    #[test]
    fn test_simple_metrics_value_struct() {
        let input = quote! {
            struct RequestValue {
                #[metrics(ignore)]
                ignore: u32,
                value: u32,
            }
        };

        // Process the input through the metrics macro
        let output = metrics_impl(input, quote!(metrics(value)));

        // Parse the output back into a syn::File for pretty printing
        let parsed_file = match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => {
                // If parsing fails, use the raw string output
                output.to_string()
            }
        };

        assert_snapshot!("simple_metrics_value_struct", parsed_file);
    }

    #[test]
    fn test_simple_metrics_value_unnamed_struct() {
        let input = quote! {
            struct RequestValue(
                #[metrics(ignore)]
                u32,
                u32);
        };

        // Process the input through the metrics macro
        let output = metrics_impl(input, quote!(metrics(value)));

        // Parse the output back into a syn::File for pretty printing
        let parsed_file = match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => {
                // If parsing fails, use the raw string output
                output.to_string()
            }
        };

        assert_snapshot!("simple_metrics_value_unnamed_struct", parsed_file);
    }

    #[test]
    fn test_simple_metrics_enum() {
        let input = quote! {
            enum Foo {
                Bar
            }
        };

        // Process the input through the metrics macro
        let output = metrics_impl(input, quote!(metrics(value(string))));

        // Parse the output back into a syn::File for pretty printing
        let parsed_file = match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => {
                // If parsing fails, use the raw string output
                output.to_string()
            }
        };

        assert_snapshot!("simple_metrics_enum", parsed_file);
    }
}
