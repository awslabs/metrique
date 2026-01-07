// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod emf;
mod entry_impl;
mod enums;
mod inflect;
mod structs;
mod value_impl;

use darling::{
    FromField, FromMeta,
    ast::NestedMeta,
    util::{Flag, SpannedValue},
};
use emf::DimensionSets;
use inflect::NameStyle;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Ts2};
use quote::{ToTokens, quote, quote_spanned};
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, Ident, Result, Type, Visibility,
    parse_macro_input, spanned::Spanned,
};

use crate::inflect::{name_contains_dot, name_contains_uninflectables, name_ends_with_delimiter};

/// Transforms a struct or enum into a unit-of-work metric.
///
/// # Container Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `rename_all` | String | Changes the case style of all field names | `#[metrics(rename_all = "PascalCase")]` |
/// | `prefix` | String | Adds a prefix to all field names (prefix gets inflected) | `#[metrics(prefix = "api_")]` |
/// | `exact_prefix` | String | Adds a prefix to all field names without inflection | `#[metrics(exact_prefix = "API_")]` |
/// | `emf::dimension_sets` | Array | Defines dimension sets for CloudWatch metrics | `#[metrics(emf::dimension_sets = [["Status", "Operation"]])]` |
/// | `tag` | Nested | On entry enums, adds a tag field with the variant name. Tag value respects `rename_all` and variant `name`, but not `prefix`. | |
/// | - `name` | String | Name of the tag field (inflectable, respects `prefix` and `rename_all`) | `#[metrics(tag(name = "operation"))]` |
/// | - `name_exact` | String | Name of the tag field (exact, not affected by `prefix` or `rename_all`) | `#[metrics(tag(name_exact = "operation"))]` |
/// | - `sample_group` | Flag | Include tag in sample group | `#[metrics(tag(name = "op", sample_group))]` |
/// | `subfield` | Flag | When set, this metric can only be used when nested within other metrics, and can be consumed by reference (has both `impl CloseValue for &MyStruct` and `impl CloseValue for MyStruct`). It cannot be added to a sink directly. | `#[metrics(subfield)]` |
/// | `subfield_owned` | Flag | When set, this metric can only be used when nested within other metrics. It cannot be added to a sink directly. | `#[metrics(subfield_owned)]` |
/// | `value` | Flag | Used for *structs*. Makes the struct a value newtype | `#[metrics(value)]` |
/// | `value(string)` | Flag | Used for *enums*. Transforms the enum into a string value. | `#[metrics(value(string))]` |
/// | `sample_group` | Flag | On `#[metrics(value)]`, forwards `sample_group` to the inner field | `#[metrics(value, sample_group)]` |
///
/// # Field Attributes
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `name` | String | Overrides the field name in metrics | `#[metrics(name = "CustomName")]` |
/// | `unit` | Path | Specifies the unit for the metric value | `#[metrics(unit = Millisecond)]` |
/// | `format` | Path | Specifies the formatter (`ValueFormatter`) for the metric value | `#[metrics(format=EpochSeconds)]` |
/// | `timestamp` | Flag | Marks a field as the canonical timestamp | `#[metrics(timestamp)]` |
/// | `sample_group` | Flag | Marks a field as a sample group - it will still be emitted as a value | `#[metrics(sample_group)]` |
/// | `prefix` | String | Adds a prefix to flattened entries. Prefix will get inflected to the right case style | `#[metrics(flatten, prefix="prefix-")]` |
/// | `exact_prefix` | String | Adds a prefix to flattened entries without inflection | `#[metrics(flatten, exact_prefix="API_")]` |
/// | `flatten` | Flag | Flattens nested `CloseEntry` metric structs | `#[metrics(flatten)]` |
/// | `flatten_entry` | Flag | Flattens nested `CloseValue<Closed: Entry>` metric structs, with no prefix or inflection | `#[metrics(flatten_entry)]` |
/// | `no_close` | Flag | Use the entry directly instead of closing it | `#[metrics(no_close)]` |
/// | `ignore` | Flag | Excludes the field from metrics | `#[metrics(ignore)]` |
///
/// # Variant Attributes
///
/// For enum usage, see the [Enums](#enums) section below.
///
/// | Attribute | Type | Description | Example |
/// |-----------|------|-------------|---------|
/// | `name` | String | Overrides the variant name in metrics | `#[metrics(name = "CustomName")]` |
///
/// # Metric Names
///
/// ## Prefixes
///
/// Prefixes can be attached to metrics in 2 different ways:
///
/// 1. Prefixes on flattened subfields, which affect all the metrics contained within
///    the flattened subfield:
///
///    ```rust
///    # use metrique::unit_of_work::metrics;
///    # use std::time::Duration;
///    #[metrics(subfield)]
///    struct Subfield {
///        request_latency: Duration, // inflected
///        #[metrics(name="NDucks")] // not inflected (since `name` is not inflected), prefixed
///        number_of_ducks: u32,
///    }
///
///    #[metrics(rename_all = "kebab-case")]
///    struct Base {
///        // uses `exact_prefix`, not inflected
///        #[metrics(flatten, exact_prefix = "API:")]
///        api: Subfield,
///        // uses `prefix`, inflected
///        #[metrics(flatten, prefix = "alt")]
///        alt: Subfield,
///    }
///
///    let vec_sink = metrique::writer::sink::VecEntrySink::new();
///    Base {
///        api: Subfield { request_latency: Duration::from_millis(1), number_of_ducks: 0 },
///        alt: Subfield { request_latency: Duration::from_millis(1), number_of_ducks: 0 }
///    }.append_on_drop(vec_sink.clone());
///    let entries = vec_sink.drain();
///    let entry = metrique::test_util::to_test_entry(&entries[0]);
///    assert_eq!(entry.metrics["API:request-latency"], 1.0);
///    assert_eq!(entry.metrics["alt-request-latency"], 1.0);
///    assert_eq!(entry.metrics["API:NDucks"], 0);
///    assert_eq!(entry.metrics["alt-NDucks"], 0);
///    ```
/// 2. Prefixes on the struct itself, which *only* affect fields within the metric
///    that don't have a `name` or a `flatten` attribute:
///
///    ```rust
///    # use metrique::unit_of_work::metrics;
///    # use std::time::Duration;
///    #[metrics(subfield)]
///    struct Subfield {
///        request_latency: Duration, // inflected
///    }
///
///    #[metrics(prefix = "Foo-" /* prefix gets inflected */, rename_all = "kebab-case")]
///    struct Base {
///        // prefix does not propagate to subfield. Use `prefix = "Foo-"` to propagate
///        #[metrics(flatten)]
///        sub: Subfield,
///        // prefix does not propagate to named field
///        #[metrics(name = "n-ducks")]
///        number_of_ducks: u32,
///        // prefix does propagate to other
///        number_of_geese: u32,
///    }
///
///    let vec_sink = metrique::writer::sink::VecEntrySink::new();
///    Base {
///        sub: Subfield { request_latency: Duration::from_millis(1) },
///        number_of_ducks: 0,
///        number_of_geese: 0
///    }.append_on_drop(vec_sink.clone());
///    let entries = vec_sink.drain();
///    let entry = metrique::test_util::to_test_entry(&entries[0]);
///    assert_eq!(entry.metrics["request-latency"], 1.0);
///    assert_eq!(entry.metrics["n-ducks"], 0);
///    // prefix-on-struct only applies to this
///    assert_eq!(entry.metrics["foo-number-of-geese"], 0);
///    ```
///
/// Note that prefix-attribute-on-flatten *does* apply to nested fields that have
/// a `name` attribute.
///
/// Prefixes can either be inflectable (with the `prefix` attribute) or non-inflectable
/// (with the `exact_prefix` attribute).
///
/// ## Inflection
///
/// Metric names are inflected to allow them to fit into the name style used by the
/// application. This uses the `Inflector` crate and supports inflecting metrics into
/// PascalCase, snake_case, and kebab-case.
///
/// Metric names assigned via the `name` attribute are not inflected, but if they are
/// contained in a metric with a prefix, the prefix can be inflected. Prefixes assigned via
/// `exact_prefix` are similarly not inflected.
///
/// For example, this emits a metric named "foo_Bar", since "Bar" is assigned via a
/// `name` attribute and therefore not inflected, but the prefix is assigned
/// via `prefix` and is therefore inflected.
///
/// ```rust
/// # use metrique::unit_of_work::metrics;
///
/// #[metrics(subfield)]
/// struct Subfield {
///     #[metrics(name = "NDucks")]
///     number_of_ducks: u32,
/// }
///
/// #[metrics(rename_all = "snake_case")]
/// struct Base {
///     #[metrics(flatten, prefix = "Waterfowl_")]
///     waterfowl: Subfield,
/// }
///
/// let vec_sink = metrique::writer::sink::VecEntrySink::new();
/// Base { waterfowl: Subfield { number_of_ducks: 0 } }
///     .append_on_drop(vec_sink.clone());
/// let entries = vec_sink.drain();
/// let entry = metrique::test_util::to_test_entry(&entries[0]);
/// assert_eq!(entry.metrics["waterfowl_NDucks"], 0);
/// ```
///
/// # Example
///
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique::timers::{Timestamp, Timer};
/// use metrique::unit::{Count, Millisecond};
/// use metrique::writer::GlobalEntrySink;
/// use metrique::ServiceMetrics;
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
///     #[metrics(sample_group)]
///     operation: Operation,
///
///     #[metrics(timestamp)]
///     timestamp: SystemTime,
///
///     #[metrics(unit = Millisecond)]
///     operation_time: Timer,
///
///     #[metrics(flatten, prefix = "sub_")]
///     nested: NestedMetrics,
///
///     request_count: RequestCount,
/// }
///
/// #[metrics(subfield)]
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
/// # Enums
///
/// Enums can be used in two ways: as value enums or entry enums.
///
/// ## Value Enums
///
/// Value enums with `#[metrics(value(string))]` convert enum variants to string values.
/// Only unit variants are allowed. Variant names respect `#[metrics(name = "...")]` and `rename_all`:
///
/// ```rust
/// # use metrique::unit_of_work::metrics;
/// #[metrics(value(string), rename_all = "snake_case")]
/// enum Operation {
///     #[metrics(name = "custom_read")]
///     ReadData,
///     WriteData,
/// }
/// // Operation::ReadData converts to "custom_read"
/// // Operation::WriteData converts to "write_data"
///
/// #[metrics]
/// struct Request {
///     #[metrics(sample_group)]
///     operation: Operation,
///     count: usize,
/// }
/// ```
///
/// ## Entry Enums
///
/// Entry enums allow different metric fields per variant. Contained fields respect container and
/// field attributes as used by structs.
///
/// Variants can be tuple variants (which must use flatten/flatten_entry/ignore attributes, since
/// their fields are unnamed). Or, they can use struct variants with named fields and the full
/// range of field attributes available. (Unit variants are also supported but don't do much unless
/// used with a `tag` field; see the following `Tag field` section.)
///
/// ```rust
/// # use metrique::unit_of_work::metrics;
/// # use metrique::unit::Millisecond;
/// # use std::time::Duration;
///
/// #[metrics(subfield)]
/// struct ReadMetrics {
///     bytes_read: usize,
/// }
///
/// #[metrics(rename_all = "PascalCase")]
/// enum Operation {
///     Read(#[metrics(flatten)] ReadMetrics),
///     Write {
///         #[metrics(unit = Millisecond)]
///         latency: Duration,
///         bytes_written: usize,
///     },
/// }
///
/// let entry = metrique::test_util::test_metric(
///     Operation::Read(ReadMetrics { bytes_read: 1024 })
/// );
/// assert_eq!(entry.metrics["BytesRead"].as_u64(), 1024);
///
/// let entry = metrique::test_util::test_metric(
///     Operation::Write { latency: Duration::from_millis(5), bytes_written: 2048 }
/// );
/// assert_eq!(entry.metrics["Latency"].as_u64(), 5);
/// assert_eq!(entry.metrics["BytesWritten"].as_u64(), 2048);
/// ```
///
/// ### Tag Field
///
/// Entry enums can include a `tag` attribute to add a field containing the variant name:
///
/// ```rust
/// # use metrique::unit_of_work::metrics;
/// # use metrique::test_util::test_metric;
/// # use metrique::unit::Millisecond;
/// # use std::time::Duration;
///
/// #[metrics(tag(name = "Operation"), rename_all = "PascalCase")]
/// enum Request {
///     Read { bytes: usize },
///     Write {
///         #[metrics(unit = Millisecond)]
///         latency: Duration,
///     },
///     // doesn't contain fields, but still has the tag field injected
///     Delete,
/// }
///
/// let entry = test_metric(Request::Read { bytes: 1024 });
/// assert_eq!(entry.values["Operation"], "Read");  // Tag field with variant name
/// assert_eq!(entry.metrics["Bytes"].as_u64(), 1024);
/// ```
///
/// The tag field name is specified explicitly and not affected by `prefix` or `rename_all`.
/// The tag value (variant name) respects `rename_all` and variant `name` attributes, but not `prefix`.
///
/// The optional `sample_group` flag includes the tag field in the sample group:
///
/// ```rust
/// # use metrique::unit_of_work::metrics;
/// # use metrique::test_util::test_metric;
///
/// #[metrics(tag(name = "Operation", sample_group))]
/// enum Request {
///     Read { bytes: usize },
///     Write { bytes: usize },
/// }
///
/// let entry = test_metric(Request::Read { bytes: 1024 });
/// // The tag field "Operation" with value "Read" is included in sample_group
/// ```
///
/// # Generated Types
///
/// For a struct or entry enum named `MyMetrics`, the macro generates:
/// - `MyMetricsEntry`: The internal representation used for serialization, implements `InflectableEntry`
/// - `MyMetricsGuard`: A wrapper that implements `Deref`/`DerefMut` to the original struct and handles emission on drop.
///   A type alias to ``AppendAndCloseOnDrop`.
/// - `MyMetricsHandle`: A shareable handle for concurrent access to the metrics.
///   A type alias to ``AppendAndCloseOnDropHandle`.
///
/// Value enums do not have new types generated, only trait implementations (`From<&MyEnum> for &'static str`, `SampleGroup`, `Value`).
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

/// Synthetic field using variant name.
#[derive(Debug, Clone)]
pub(crate) enum Tag {
    Inflectable { name: String, sample_group: bool },
    Exact { name: String, sample_group: bool },
}

impl Tag {
    /// Get the tag field name, applying inflection if using inflectable variant
    pub(crate) fn field_name(&self, root_attrs: &RootAttributes) -> String {
        match self {
            Tag::Inflectable { name, .. } => root_attrs
                .prefix
                .as_ref()
                .map(|p| p.apply(name, root_attrs.rename_all))
                .unwrap_or_else(|| root_attrs.rename_all.apply(name)),
            Tag::Exact { name, .. } => name.clone(),
        }
    }

    pub(crate) fn sample_group(&self) -> bool {
        match self {
            Tag::Inflectable { sample_group, .. } => *sample_group,
            Tag::Exact { sample_group, .. } => *sample_group,
        }
    }
}

#[derive(Debug, FromMeta)]
#[darling(and_then = Self::validate, from_word = Self::from_word)]
struct RawTag {
    #[darling(default)]
    name: Option<SpannedKv<String>>,
    #[darling(default)]
    name_exact: Option<SpannedKv<String>>,
    #[darling(default)]
    sample_group: Flag,
}

impl RawTag {
    fn from_word() -> darling::Result<Self> {
        Err(darling::Error::custom(
            "tag requires either name or name_exact parameter: #[metrics(tag(name = \"...\"))] or #[metrics(tag(name_exact = \"...\"))]",
        ))
    }

    fn validate(self) -> darling::Result<Self> {
        match (self.name, self.name_exact) {
            (None, None) => Err(darling::Error::custom(
                "tag requires either name or name_exact parameter: #[metrics(tag(name = \"...\"))] or #[metrics(tag(name_exact = \"...\"))]",
            )),
            (Some(_), Some(_)) => Err(darling::Error::custom(
                "tag cannot have both name and name_exact parameters",
            )),
            (Some(name), None) => Ok(Self {
                name: Some(validate_name(name)?),
                name_exact: None,
                sample_group: self.sample_group,
            }),
            (None, Some(name_exact)) => Ok(Self {
                name: None,
                name_exact: Some(validate_name(name_exact)?),
                sample_group: self.sample_group,
            }),
        }
    }
}

impl From<RawTag> for Tag {
    fn from(raw: RawTag) -> Self {
        let sample_group = raw.sample_group.is_present();
        match (raw.name, raw.name_exact) {
            (Some(name), None) => Tag::Inflectable {
                name: name.value,
                sample_group,
            },
            (None, Some(name)) => Tag::Exact {
                name: name.value,
                sample_group,
            },
            _ => unreachable!("validated in RawTag::validate"),
        }
    }
}

#[derive(Debug, Default, FromMeta)]
struct RawRootAttributes {
    prefix: Option<SpannedKv<String>>,
    exact_prefix: Option<SpannedKv<String>>,

    #[darling(default)]
    rename_all: NameStyle,

    #[darling(rename = "emf::dimension_sets")]
    emf_dimensions: Option<DimensionSets>,

    tag: Option<SpannedValue<RawTag>>,

    subfield: Flag,
    #[darling(rename = "subfield_owned")]
    subfield_owned: Flag,
    #[darling(rename = "sample_group")]
    sample_group: Flag,
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
    prefix: Option<Prefix>,

    rename_all: NameStyle,

    emf_dimensions: Option<DimensionSets>,

    tag: Option<Tag>,

    sample_group: bool,

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
        let mut mode = out.map(|(s, _)| s).unwrap_or_default();
        let sample_group = if self.sample_group.is_present() {
            if let MetricMode::Value = &mut mode {
                true
            } else {
                return Err(darling::Error::custom(
                    "`sample_group` as a top-level attribute can only be used with #[metrics(value)]",
                )
                .with_span(&self.sample_group.span()));
            }
        } else {
            false
        };
        if let (MetricMode::ValueString, Some(ds)) = (mode, &self.emf_dimensions) {
            return Err(
                darling::Error::custom("value does not make sense with dimension-sets")
                    .with_span(&ds.span()),
            );
        }
        let tag = self
            .tag
            .map(|tag| match &mode {
                MetricMode::RootEntry | MetricMode::Subfield | MetricMode::SubfieldOwned => {
                    Ok(tag.into_inner().into())
                }
                MetricMode::Value | MetricMode::ValueString => Err(darling::Error::custom(
                    "value and value(string) do not support tag",
                )
                .with_span(&tag.span())),
            })
            .transpose()?;

        Ok(RootAttributes {
            prefix: Prefix::from_inflectable_and_exact(
                &self.prefix,
                &self.exact_prefix,
                PrefixLevel::Root,
            )?
            .map(SpannedValue::into_inner),
            rename_all: self.rename_all,
            emf_dimensions: self.emf_dimensions,
            tag,
            sample_group,
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

    fn warnings(&self) -> Ts2 {
        quote! {}
    }
}

#[derive(Debug, FromField)]
#[darling(attributes(metrics))]
struct RawMetricsFieldAttrs {
    flatten: Flag,

    flatten_entry: Flag,

    no_close: Flag,

    timestamp: Flag,

    sample_group: Flag,

    ignore: Flag,

    #[darling(default)]
    unit: Option<SpannedKv<syn::Path>>,

    #[darling(default)]
    format: Option<SpannedKv<syn::Path>>,

    #[darling(default)]
    name: Option<SpannedKv<String>>,

    #[darling(default)]
    prefix: Option<SpannedKv<String>>,

    #[darling(default)]
    exact_prefix: Option<SpannedKv<String>>,
}

/// Wrapper type to allow recovering both the key and value span when parsing an attribute
#[derive(Debug)]
pub(crate) struct SpannedKv<T> {
    pub(crate) key_span: Span,
    #[allow(dead_code)]
    pub(crate) value_span: Span,
    pub(crate) value: T,
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

pub(crate) fn parse_metric_fields(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> Result<Vec<MetricsField>> {
    let mut parsed_fields = vec![];
    let mut errors = darling::Error::accumulator();

    for (i, field) in fields.iter().enumerate() {
        let i = syn::Index::from(i);
        let (ident, name, span) = match &field.ident {
            Some(ident) => (quote! { #ident }, Some(ident.to_string()), ident.span()),
            None => (quote! { #i }, None, field.ty.span()),
        };

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

fn cannot_combine_error(existing: &str, new: &str, new_span: Span) -> darling::Error {
    darling::Error::custom(format!("Cannot combine `{existing}` with `{new}`")).with_span(&new_span)
}

// Set metrics to `new`, enforcing the fact that this field is exclusive and cannot be combined
fn set_exclusive<T>(
    new: impl Fn(Span) -> T,
    name: &'static str,
    existing: Option<(T, &'static str)>,
    flag: &Flag,
) -> darling::Result<Option<(T, &'static str)>> {
    match (flag.is_present(), &existing) {
        (true, Some((_, other))) => Err(cannot_combine_error(other, name, flag.span())),
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
        (Some(input), Some((_, other))) => {
            Err(cannot_combine_error(other, field_name, input.key_span))
        }
        (Some(v), None) => Ok(Some(&v.value)),
        _ => Ok(None),
    }
}

// retrieve the value for a flag that requires a value to be a field
fn get_field_flag(
    field_name: &'static str,
    existing: &Option<(MetricsFieldKind, &'static str)>,
    flag: &Flag,
) -> darling::Result<Option<Span>> {
    match (flag.is_present(), &existing) {
        (true, Some((_, other))) => Err(cannot_combine_error(other, field_name, flag.span())),
        (true, None) => Ok(Some(flag.span())),
        _ => Ok(None),
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
        let sample_group = get_field_flag("sample_group", &out, &self.sample_group)?;
        let close = !self.no_close.is_present();
        if let (false, Some((MetricsFieldKind::Ignore(span), _))) = (close, &out) {
            return Err(cannot_combine_error("no_close", "ignore", *span));
        }

        let prefix = Prefix::from_inflectable_and_exact(
            &self.prefix,
            &self.exact_prefix,
            PrefixLevel::Field,
        )?;
        if let Some(prefix_) = prefix {
            match &mut out {
                Some((MetricsFieldKind::Flatten { prefix, .. }, _)) => {
                    *prefix = Some(prefix_.into_inner());
                }
                _ => {
                    return Err(
                        darling::Error::custom("prefix can only be used with `flatten`")
                            .with_span(&prefix_.span()),
                    );
                }
            }
        }

        Ok(MetricsFieldAttrs {
            close,
            kind: match out {
                Some((out, _)) => out,
                None => MetricsFieldKind::Field {
                    sample_group,
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

#[derive(Debug, Clone)]
struct MetricsFieldAttrs {
    close: bool,
    kind: MetricsFieldKind,
}

pub(crate) struct MetricsField {
    pub(crate) vis: Visibility,
    pub(crate) ident: Ts2,
    pub(crate) name: Option<String>,
    pub(crate) span: Span,
    pub(crate) ty: Type,
    pub(crate) external_attrs: Vec<Attribute>,
    pub(crate) attrs: MetricsFieldAttrs,
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
                #[deprecated(note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`")]
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

    pub(crate) fn close_value(&self, ownership_kind: OwnershipKind) -> Ts2 {
        let ident = &self.ident;
        let span = self.span;
        let field_expr = match ownership_kind {
            OwnershipKind::ByValue => quote_spanned! {span=> self.#ident },
            OwnershipKind::ByRef => quote_spanned! {span=> &self.#ident },
        };
        self.close_field_expr(field_expr)
    }

    pub(crate) fn close_field_expr(&self, field_expr: Ts2) -> Ts2 {
        let ident = &self.ident;
        let span = self.span;
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

pub(crate) struct TupleData {
    pub(crate) ty: syn::Type,
    pub(crate) kind: MetricsFieldKind,
    pub(crate) close: bool,
}

/// Generate the entry type for a field/variant, optionally closing it
pub(crate) fn entry_type(ty: &syn::Type, close: bool, span: proc_macro2::Span) -> Ts2 {
    if close {
        quote::quote_spanned! { span=> <#ty as metrique::CloseValue>::Closed }
    } else {
        quote::quote_spanned! { span=> #ty }
    }
}

pub(crate) enum PrefixLevel {
    Root,
    Field,
}

#[derive(Debug, Clone)]
pub(crate) enum Prefix {
    Inflectable { prefix: String },
    Exact(String),
}

impl Prefix {
    /// Apply prefix to base name and inflect according to name_style
    pub(crate) fn apply(&self, base: &str, name_style: NameStyle) -> String {
        match self {
            Prefix::Exact(exact_prefix) => {
                format!("{}{}", exact_prefix, name_style.apply(base))
            }
            Prefix::Inflectable { prefix } => {
                let prefixed = format!("{}{}", prefix, base);
                name_style.apply(&prefixed)
            }
        }
    }

    fn inflected_prefix_message(prefix: &str, c: char) -> String {
        let warning_text = if name_contains_dot(prefix) {
            " '.' used to be allowed in `prefix` but is now forbidden."
        } else {
            ""
        };
        let prefix_fixed: String = prefix
            .chars()
            .map(|c| if !c.is_alphanumeric() { '-' } else { c })
            .collect();
        format!(
            "You cannot use the character {c:?} with `prefix`. `prefix` will \"inflect\" to match the name scheme specified by `rename_all`. For example, \
            it will change all delimiters to `-` for kebab case). If you want to match namestyle, use `prefix = {prefix_fixed:?}`. If you want to preserve {c:?} \
            in the final metric name use `exact_prefix = {prefix:?}.{warning_text}"
        )
    }

    fn prefix_should_end_with_delimiter_message(prefix: &str) -> String {
        let delimiter = if prefix.contains('-') { '-' } else { '_' };
        let prefix_fixed = format!("{prefix}{delimiter}");
        format!(
            "The root-level prefix `{prefix:?}` must end with a delimiter. Use `prefix = {prefix_fixed:?}`, which inflects \
            correctly in all inflections"
        )
    }

    fn from_inflectable_and_exact(
        inflectable: &Option<SpannedKv<String>>,
        exact: &Option<SpannedKv<String>>,
        level: PrefixLevel,
    ) -> darling::Result<Option<SpannedValue<Self>>> {
        match (inflectable, exact) {
            (Some(prefix), None) => {
                if let Some(c) = name_contains_uninflectables(&prefix.value) {
                    Err(
                        darling::Error::custom(Self::inflected_prefix_message(&prefix.value, c))
                            .with_span(&prefix.key_span),
                    )
                } else if let PrefixLevel::Root = level
                    && !name_ends_with_delimiter(&prefix.value)
                {
                    Err(
                        darling::Error::custom(Self::prefix_should_end_with_delimiter_message(
                            &prefix.value,
                        ))
                        .with_span(&prefix.key_span),
                    )
                } else {
                    Ok(Some(SpannedValue::new(
                        Self::Inflectable {
                            prefix: prefix.value.clone(),
                        },
                        prefix.key_span,
                    )))
                }
            }
            (None, Some(p)) => Ok(Some(SpannedValue::new(
                Prefix::Exact(p.value.clone()),
                p.key_span,
            ))),
            (None, None) => Ok(None),
            (Some(inflectable), Some(_)) => Err(cannot_combine_error(
                "prefix",
                "exact_prefix",
                inflectable.key_span,
            )),
        }
    }

    /// Append this prefix to the namespace's prefix chain.
    /// The prefix will be prepended to metric names when they are written.
    /// Returns (extra, namespace_with_prefix).
    pub(crate) fn append_to(
        &self,
        ns: &proc_macro2::TokenStream,
        span: proc_macro2::Span,
    ) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
        match self {
            Prefix::Inflectable { prefix } => {
                crate::entry_impl::make_inflect_prefix(ns, prefix, span)
            }
            Prefix::Exact(exact_prefix) => {
                crate::entry_impl::make_exact_prefix(ns, exact_prefix, span)
            }
        }
    }
}

#[derive(Debug, Clone)]
enum MetricsFieldKind {
    Ignore(Span),
    Flatten {
        span: Span,
        prefix: Option<Prefix>,
    },
    FlattenEntry(Span),
    Timestamp(Span),
    Field {
        unit: Option<syn::Path>,
        name: Option<String>,
        format: Option<syn::Path>,
        sample_group: Option<Span>,
    },
}

// produce a warning that the user can see
//
// currently, we do not have any logic that produces warnings, but leave this
// in for the next time
#[allow(unused)]
fn proc_macro_warning(span: Span, warning: &str) -> Ts2 {
    quote_spanned! {span=>
        const _: () = {
            #[deprecated(note=#warning)]
            const _W: () = ();
            _W
        };
    }
}

fn parse_root_attrs(attr: TokenStream) -> Result<RootAttributes> {
    let nested_meta = NestedMeta::parse_meta_list(attr.into())?;
    Ok(RawRootAttributes::from_list(&nested_meta)?.validate()?)
}

fn generate_metrics(root_attributes: RootAttributes, input: DeriveInput) -> Result<Ts2> {
    let output = match root_attributes.mode {
        MetricMode::RootEntry | MetricMode::Subfield | MetricMode::SubfieldOwned => {
            match &input.data {
                Data::Struct(data_struct) => {
                    if root_attributes.tag.is_some() {
                        return Err(Error::new_spanned(
                            &input,
                            "`tag` attribute is only supported on entry enums",
                        ));
                    }
                    let fields = match &data_struct.fields {
                        Fields::Named(fields_named) => &fields_named.named,
                        _ => {
                            return Err(Error::new_spanned(
                                &input,
                                "Only named fields are supported",
                            ));
                        }
                    };
                    structs::generate_metrics_for_struct(root_attributes, &input, fields)?
                }
                Data::Enum(data_enum) => {
                    let variants =
                        enums::parse_enum_variants(&data_enum.variants, enums::VariantMode::Entry)?;
                    enums::generate_metrics_for_enum(root_attributes, &input, &variants)?
                }
                Data::Union(_) => {
                    return Err(Error::new_spanned(
                        &input,
                        "Only structs and enums are supported for entries",
                    ));
                }
            }
        }
        MetricMode::Value => match &input.data {
            Data::Struct(data_struct) => {
                let fields = match &data_struct.fields {
                    Fields::Named(fields_named) => &fields_named.named,
                    Fields::Unnamed(fields_unnamed) => &fields_unnamed.unnamed,
                    _ => {
                        return Err(Error::new_spanned(
                            &input,
                            "Only named fields are supported",
                        ));
                    }
                };
                structs::generate_metrics_for_struct(root_attributes, &input, fields)?
            }
            _ => {
                return Err(Error::new_spanned(
                    &input,
                    "Only structs are supported with value, use value(string) with enums",
                ));
            }
        },
        MetricMode::ValueString => {
            let variants = match &input.data {
                Data::Enum(data_enum) => &data_enum.variants,
                _ => {
                    return Err(Error::new_spanned(
                        &input,
                        "Only enums are supported with value(string)",
                    ));
                }
            };
            let variants = enums::parse_enum_variants(variants, enums::VariantMode::ValueString)?;
            enums::generate_metrics_for_enum(root_attributes, &input, &variants)?
        }
    };

    if std::env::var("MACRO_DEBUG").is_ok() {
        eprintln!("{}", &output);
    }

    Ok(output)
}

/// Generate the on_drop_wrapper implementation
pub(crate) fn generate_on_drop_wrapper(
    vis: &Visibility,
    guard: &Ident,
    inner: &Ident,
    target: &Ident,
    handle: &Ident,
) -> Ts2 {
    let inner_str = inner.to_string();
    let guard_str = guard.to_string();
    quote! {
        #[doc = concat!("Metrics guard returned from [`", #inner_str, "::append_on_drop`], closes the entry and appends the metrics to a sink when dropped.")]
        #vis type #guard<Q = ::metrique::DefaultSink> = ::metrique::AppendAndCloseOnDrop<#inner, Q>;
        #[doc = concat!("Metrics handle returned from [`", #guard_str, "::handle`], similar to an `Arc<", #guard_str, ">`.")]
        #vis type #handle<Q = ::metrique::DefaultSink> = ::metrique::AppendAndCloseOnDropHandle<#inner, Q>;

        impl #inner {
            #[doc = "Creates an AppendAndCloseOnDrop that will be automatically appended to `sink` on drop."]
            #vis fn append_on_drop<Q: ::metrique::writer::EntrySink<::metrique::RootEntry<#target>> + Send + Sync + 'static>(self, sink: Q) -> #guard<Q> {
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

pub(crate) fn clean_attrs(attr: &[Attribute]) -> Vec<Attribute> {
    attr.iter()
        .filter(|attr| !attr.path().is_ident("metrics"))
        .cloned()
        .collect()
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
                structs::clean_base_struct(vis, adt_name, generics, filtered_attrs, fields_named)
            }
            Fields::Unnamed(fields_unnamed) => structs::clean_base_unnamed_struct(
                vis,
                adt_name,
                generics,
                filtered_attrs,
                fields_unnamed,
            ),
            // In these cases, we can't strip attributes since we don't support this format.
            // Echo back exactly what was given.
            _ => input.to_token_stream(),
        },
        Data::Enum(data_enum) => {
            if let Ok(variants) = enums::parse_enum_variants(
                &data_enum.variants,
                enums::VariantMode::SkipAttributeParsing,
            ) {
                enums::generate_base_enum(adt_name, vis, generics, &filtered_attrs, &variants)
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

    fn metrics_impl_string(input: Ts2, attrs: Ts2) -> String {
        let output = metrics_impl(input, attrs);

        // Parse the output back into a syn::File for pretty printing
        match parse2::<syn::File>(output.clone()) {
            Ok(file) => prettyplease::unparse(&file),
            Err(_) => {
                // If parsing fails, print the error and use the raw string output
                output.to_string()
            }
        }
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

        let parsed_file = metrics_impl_string(input, quote!(metrics()));
        assert_snapshot!("simple_metrics_struct", parsed_file);
    }

    #[test]
    fn test_sample_group_metrics_struct() {
        let input = quote! {
            struct RequestMetrics {
                #[metrics(sample_group)]
                operation: &'static str,
                number_of_ducks: usize
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics()));
        assert_snapshot!("sample_group_metrics_struct", parsed_file);
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

        let parsed_file = metrics_impl_string(input, quote!(metrics(value)));
        assert_snapshot!("simple_metrics_value_struct", parsed_file);
    }

    #[test]
    fn test_sample_group_metrics_value_struct() {
        let input = quote! {
            struct RequestValue {
                #[metrics(ignore)]
                ignore: u32,
                value: &'static str,
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics(value, sample_group)));
        assert_snapshot!("sample_group_metrics_value_struct", parsed_file);
    }

    #[test]
    fn test_simple_metrics_value_unnamed_struct() {
        let input = quote! {
            struct RequestValue(
                #[metrics(ignore)]
                u32,
                u32);
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics(value)));
        assert_snapshot!("simple_metrics_value_unnamed_struct", parsed_file);
    }

    #[test]
    fn test_simple_metrics_enum() {
        let input = quote! {
            enum Foo {
                Bar
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics(value(string))));
        assert_snapshot!("simple_metrics_enum", parsed_file);
    }

    #[test]
    fn test_exact_prefix_struct() {
        let input = quote! {
            struct RequestMetrics {
                operation: &'static str,
                number_of_ducks: usize
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics(exact_prefix = "API@")));
        assert_snapshot!("exact_prefix_struct", parsed_file);
    }

    #[test]
    fn test_field_exact_prefix_struct() {
        let input = quote! {
            struct RequestMetrics {
                #[metrics(flatten, exact_prefix = "API@")]
                nested: NestedMetrics,
                operation: &'static str
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics()));
        assert_snapshot!("field_exact_prefix_struct", parsed_file);
    }

    #[test]
    fn test_field_inflectable_prefix_struct() {
        let input = quote! {
            struct RequestMetrics {
                #[metrics(flatten, prefix = "api_")]
                nested: NestedMetrics,
                operation: &'static str
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics()));
        assert_snapshot!("field_inflectable_prefix_struct", parsed_file);
    }

    #[test]
    fn test_entry_enum() {
        let nested = metrics_impl_string(
            quote! {
                #[metrics(subfield)]
                struct Nested {
                    value: u32,
                }
            },
            quote!(metrics(subfield)),
        );
        let status = metrics_impl_string(
            quote! {
                #[metrics(subfield)]
                enum Status {
                    Active {
                        count: u32,
                        #[metrics(unit = metrique::writer::unit::Millisecond)]
                        latency: u64,
                    },
                    Pending(#[metrics(flatten)] Nested),
                    Multi(
                        #[metrics(flatten)] Nested,
                        #[metrics(ignore)] u32,
                    ),
                }
            },
            quote!(metrics(subfield)),
        );
        let root = metrics_impl_string(
            quote! {
                enum Operation {
                    Read { bytes: u64 },
                    Write(#[metrics(flatten)] Nested),
                }
            },
            quote!(metrics()),
        );

        let parsed_file = format!("{}\n{}\n{}", nested, status, root);
        assert_snapshot!("entry_enum", parsed_file);
    }

    #[test]
    fn test_subfield_struct() {
        let input = quote! {
            #[metrics(subfield)]
            struct NestedMetrics {
                counter: u32,
            }
        };

        let parsed_file = metrics_impl_string(input, quote!(metrics(subfield)));
        assert_snapshot!("subfield_struct", parsed_file);
    }

    #[test]
    fn test_sample_group_entry_enum() {
        let operation = metrics_impl_string(
            quote! {
                #[metrics(value(string))]
                enum Operation {
                    Read,
                    Write,
                }
            },
            quote!(metrics(value(string))),
        );
        let metadata = metrics_impl_string(
            quote! {
                #[metrics(subfield)]
                struct Metadata {
                    #[metrics(sample_group)]
                    operation: Operation,
                    request_id: String,
                }
            },
            quote!(metrics(subfield)),
        );
        let result = metrics_impl_string(
            quote! {
                enum RequestResult {
                    Success {
                        #[metrics(sample_group)]
                        operation: Operation,
                        bytes: usize,
                    },
                    Error {
                        #[metrics(sample_group)]
                        operation: Operation,
                        error_code: u32,
                    },
                    Timeout(#[metrics(flatten)] Metadata),
                    Cancelled(
                        #[metrics(flatten)] Metadata,
                        #[metrics(flatten_entry, no_close)] StatusEntry
                    ),
                }
            },
            quote!(metrics()),
        );

        let parsed_file = format!("{}\n{}\n{}", operation, metadata, result);
        assert_snapshot!("sample_group_entry_enum", parsed_file);
    }

    #[test]
    fn test_entry_enum_tag() {
        let nested = metrics_impl_string(
            quote! {
                #[metrics(subfield)]
                struct Nested {
                    value: u32,
                }
            },
            quote!(metrics(subfield)),
        );
        let root = metrics_impl_string(
            quote! {
                #[metrics(tag(name = "operation"))]
                enum Operation {
                    Read { bytes: usize },
                    Write(#[metrics(flatten)] Nested),
                }
            },
            quote!(metrics(tag(name = "operation"))),
        );

        let parsed_file = format!("{}\n{}", nested, root);
        assert_snapshot!("entry_enum_tag", parsed_file);
    }

    #[test]
    fn test_entry_enum_tag_with_sample_group() {
        let root = metrics_impl_string(
            quote! {
                #[metrics(tag(name = "operation", sample_group))]
                enum Operation {
                    Read { bytes: usize },
                }
            },
            quote!(metrics(tag(name = "operation", sample_group))),
        );

        assert_snapshot!("entry_enum_tag_sample_group", root);
    }
}
