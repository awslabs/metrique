// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::collections::HashSet;

use darling::{FromAttributes as _, util::SpannedValue};
use proc_macro2::{Literal, Span, TokenStream};
use quote::{ToTokens, quote, quote_spanned};
use syn::{Attribute, Path, spanned::Spanned};
use synstructure::{BindingInfo, Structure, VariantInfo};

macro_rules! decl_derive {
    ($name:ident, $derive_fn:ident) => {
synstructure::decl_derive!([$name, attributes(entry)] =>
    /// Derive `Entry` for a struct or enum.
    ///
    /// Each field in the struct or enum variant will be written to the metric entry using the rust field name by
    /// default. For example,
    /// ```ignore
    /// #[derive(Entry)]
    /// struct MySimpleEntry {
    ///     first: String,
    ///     second: u64,
    ///     third: Option<bool>,
    /// }
    /// ```
    /// will impl `Entry` like
    /// ```ignore
    /// impl Entry for MySimpleEntry {
    ///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
    ///         writer.value("first", &self.first);
    ///         writer.value("second", &self.second);
    ///         writer.value("third", &self.third);
    ///     }
    /// }
    /// ```
    ///
    /// # Container attributes
    ///
    /// The `#[entry]` attribute can be attached to structs or enums to customize the following:
    ///  * `#[entry(rename_all = {case})]` to rename all fields in the given `case` pattern. This is helpful when the
    ///    expected metric name pattern doesn't match Rust's default `snake_case`. `case` can be one of
    ///    * `"lowercase"`
    ///    * `"UPPERCASE"`
    ///    * `"PascalCase"`
    ///    * `"camelCase"`
    ///    * `"snake_case"`
    ///    * `"SCREAMING_SNAKE_CASE"`
    ///    * `"kebab-case"`
    ///    * `"SCREAMING-KEBAB-CASE"`
    ///
    /// # Field attributes
    ///
    /// The `#[entry]` attribute can be attached to fields of structs, tuples, and enums to customize the following:
    ///  * `#[entry(name = "{name}")]` to override the default name (including any case changes from `rename_all`)
    ///  * `#[entry(ignore)]` to not write the field to the metrics entry
    ///  * `#[entry(flatten)]` to treat the field as a sub-entry whose contents will be merged with the current entry.
    ///    Note that any `sample_group` will be concatenated to this entry's!
    ///  * `#[entry(timestamp)]` to treat the field as the entry's timestamp. Note that it must impl
    ///    `Into<SystemTime>`!
    ///  * `#[entry(sample_group)]` to treat the field as part of the entry's `sample_group`. The field's name (
    ///     optionally overwritten by the `name` attribute) will be used as the key. Note that the field value must be
    ///     cloneable and impl `Into<Cow<'static, str>>`!
    ///  * `#[entry(format = FORMATTER)]` to format the field using a custom format, which should be a type
    ///    implementing `ValueFormatter`.
    ///
    /// # Enums
    ///
    /// Each enum variant is treated as if it was a separate metric entry. This is useful when multiple, distinct
    /// operations can be output to the same sink! For example,
    /// ```ignore
    /// #[derive(Entry)]
    /// enum Operation {
    ///     Simple {
    ///         count: u64,
    ///     }
    ///     Read(#[entry(flatten)] ReadEntry),
    ///     Write(#[entry(flatten)] WriteEntry),
    /// }
    ///
    /// #[derive(Entry)]
    /// struct ReadEntry; // ...
    ///
    /// #[derive(Entry)]
    /// struct WriteEntry; // ...
    /// ```
    /// will impl `Entry` like
    /// ```ignore
    /// impl Entry for Operation {
    ///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
    ///         match self {
    ///             Self::Simple { count } => {
    ///                 writer.value("count", count);
    ///             }
    ///             Self::Read(read) => {
    ///                 read.write(writer);
    ///             }
    ///             Self::Write(write) => {
    ///                 write.write(writer);
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # "Real" example
    ///
    /// A real AWS service outputting EMF metrics might have a set of metric structs like
    /// ```ignore
    /// // Common constants across all entries, to be used with EntryIoStream::merge_globals()
    /// // Operation-specific metrics
    /// #[derive(Entry)]
    /// enum OperationMetrics {
    ///     Foo(#[entry(flatten)] FooMetrics),
    ///     Bar(#[entry(flatten)] BarMetrics),
    /// }
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct FooMetrics {
    ///     #[entry(sample_group)]
    ///     operation: &'static str,
    ///     success: bool,
    ///     retries: u32,
    ///     remote_call: Option<Duration>,
    /// }
    /// ```
    $derive_fn
);
    }
}

decl_derive!(Entry, derive_entry);
decl_derive!(MetriqueEntry, derive_metrique_entry);

fn derive_entry(input: Structure<'_>) -> TokenStream {
    tokens_or_compiler_err(try_derive(input, &quote!(::metrique_writer)))
}

fn derive_metrique_entry(input: Structure<'_>) -> TokenStream {
    tokens_or_compiler_err(try_derive(input, &quote!(::metrique::writer)))
}

// Raw per-field attributes for #[entry]
#[derive(darling::FromAttributes)]
#[darling(attributes(entry))]
struct ParsedFieldMetricAttr {
    name: Option<SpannedValue<String>>,
    sample_group: Option<SpannedValue<()>>,
    ignore: Option<SpannedValue<()>>,
    flatten: Option<SpannedValue<()>>,
    timestamp: Option<SpannedValue<()>>,
    format: Option<SpannedValue<Path>>,
}

// Validated per-field attributes
enum FieldMetricAttr {
    Ignore,
    Flatten,
    Timestamp(Span),
    NamedValue {
        name: Option<SpannedValue<String>>,
        format: Option<SpannedValue<Path>>,
        sample_group: Option<Span>,
    },
}

impl FieldMetricAttr {
    fn try_parse(field_span: Span, attrs: &[Attribute]) -> syn::Result<Self> {
        match ParsedFieldMetricAttr::from_attributes(attrs)? {
            ParsedFieldMetricAttr {
                name,
                sample_group,
                format,
                ignore: None,
                flatten: None,
                timestamp: None,
            } => {
                if let Some(name) = name.as_ref() {
                    if name.is_empty() {
                        return Err(syn::Error::new(name.span(), "`name` can't be empty"));
                    }
                }
                Ok(Self::NamedValue {
                    name,
                    sample_group: sample_group.map(|g| g.span()),
                    format,
                })
            }

            ParsedFieldMetricAttr {
                name: None,
                sample_group: None,
                ignore: Some(_ignore),
                flatten: None,
                timestamp: None,
                format: None,
            } => Ok(Self::Ignore),

            ParsedFieldMetricAttr {
                name: None,
                sample_group: None,
                ignore: None,
                flatten: Some(_flatten),
                timestamp: None,
                format: None,
            } => Ok(Self::Flatten),

            ParsedFieldMetricAttr {
                name: None,
                sample_group: None,
                ignore: None,
                flatten: None,
                timestamp: Some(timestamp),
                format: None,
            } => Ok(Self::Timestamp(timestamp.span())),

            _ => Err(syn::Error::new(
                field_span,
                "can only combine `name` and `sample_group` in `#[entry]`",
            )),
        }
    }
}

// Container-level attributes for #[entry]
#[derive(darling::FromAttributes)]
#[darling(attributes(entry))]
struct ContainerMetricAttr {
    rename_all: Option<SpannedValue<String>>,
}

impl ContainerMetricAttr {
    fn merge_with_defaults_from(self, root: &Self) -> Self {
        Self {
            rename_all: self.rename_all.or_else(|| root.rename_all.clone()),
        }
    }
}

fn try_derive(input: Structure<'_>, krate: &TokenStream) -> syn::Result<TokenStream> {
    let span = input.ast().span();

    let container_attr = match &input.ast().data {
        syn::Data::Struct(_) | syn::Data::Enum(_) => {
            ContainerMetricAttr::from_attributes(&input.ast().attrs)?
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new(span, "can't derive `Entry` for unions"));
        }
    };

    let mut writes = Vec::new();
    let mut sample_groups = Vec::new();
    let has_multiple_variants = input.variants().len() > 1;
    for variant in input.variants() {
        let EntryVariant {
            write,
            sample_group,
        } = derive_variant(variant, &container_attr, has_multiple_variants, krate)?;
        writes.push(write);
        sample_groups.push(sample_group);
    }

    Ok(input.gen_impl(quote_spanned! {span=>
        gen impl #krate::core::entry::Entry for @Self {
            fn write<'a>(&'a self, writer: &mut impl #krate::core::entry::EntryWriter<'a>) {
                match *self {
                    #(#writes)*
                }
            }

            fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                match *self {
                    #(#sample_groups)*
                }
            }
        }
    }))
}

fn tokens_or_compiler_err(result: syn::Result<TokenStream>) -> TokenStream {
    match result {
        Ok(t) => t,
        Err(e) => e.into_compile_error(),
    }
}

/// Return an iterator that chains the iterators in `iterators`.
///
/// This calls `chain` in a binary tree fashion to avoid problems with the recursion limit,
/// e.g. `I1.chain(I2).chain(I3.chain(I4))`
fn make_binary_tree_chain(iterators: Vec<TokenStream>) -> TokenStream {
    fn chain_once(stack: &mut Vec<(TokenStream, usize)>, allow_different_degree: bool) -> bool {
        if stack.len() < 2 {
            return false; // can't merge a stack of length < 2
        }
        if !allow_different_degree && (stack[stack.len() - 2].1 != stack[stack.len() - 1].1) {
            return false; // not merging elements of different degree if not wanted
        }
        let (rhs, rhs_deg) = stack.pop().unwrap();
        let (lhs, lhs_deg) = stack.pop().unwrap();
        stack.push((
            quote!(#lhs.chain(#rhs)),
            std::cmp::max(lhs_deg, rhs_deg) + 1,
        ));
        true
    }

    let mut stack = vec![];
    for elem in iterators {
        stack.push((elem, 0));
        while chain_once(&mut stack, false) {}
    }
    while chain_once(&mut stack, true) {}
    if let Some((elem, _deg)) = stack.pop() {
        elem
    } else {
        quote!(::std::iter::empty())
    }
}

fn derive_variant(
    variant: &VariantInfo,
    root_container_attr: &ContainerMetricAttr,
    has_multiple_variants: bool,
    krate: &TokenStream,
) -> syn::Result<EntryVariant> {
    let container_attr = ContainerMetricAttr::from_attributes(variant.ast().attrs)?
        .merge_with_defaults_from(root_container_attr);

    let mut fields = FieldSet {
        namer: Namer {
            rename_all: container_attr
                .rename_all
                .map_or(Ok(None), |r| NameStyle::try_parse(r.span(), &r).map(Some))?,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut errors: Vec<syn::Error> = variant
        .bindings()
        .iter()
        .flat_map(|field| fields.add(field, krate).err())
        .collect();
    if let Some(mut error) = errors.pop() {
        // flatten any errors past the first into the first error
        error.extend(errors);
        Err(error)
    } else {
        let pat = variant.pat();
        let FieldSet {
            writes,
            sample_groups,
            ..
        } = fields;

        let write = quote!(#pat => { #(#writes)* });
        let sample_group_iter = make_binary_tree_chain(sample_groups);
        let sample_group = if has_multiple_variants {
            // Without boxing, each variant will have a different iterator type and therefore wouldn't compile. Boxing
            // coerces all of them into Box<dyn Iterator>. In the future, we could optimize this by introducing an
            // iterator enum with one variant per iterator type (like itertool's Either).
            quote!(#pat => Box::new(#sample_group_iter) as Box<dyn ::std::iter::Iterator<Item = _>>,)
        } else {
            quote!(#pat => #sample_group_iter,)
        };
        Ok(EntryVariant {
            write,
            sample_group,
        })
    }
}

struct EntryVariant {
    write: TokenStream,
    sample_group: TokenStream,
}

#[derive(Default)]
struct FieldSet {
    namer: Namer,
    has_timestamp: bool,
    writes: Vec<TokenStream>,
    sample_groups: Vec<TokenStream>,
}

impl FieldSet {
    fn add(&mut self, field: &BindingInfo<'_>, krate: &TokenStream) -> syn::Result<()> {
        match FieldMetricAttr::try_parse(field.span(), &field.ast().attrs)? {
            FieldMetricAttr::NamedValue {
                name,
                sample_group,
                format,
            } => {
                let name = Literal::string(&if let Some(name) = name {
                    self.namer.specified(&name)?
                } else {
                    self.namer.unspecified(field)?
                });

                let field_tokens: TokenStream = match format {
                    None => field.to_token_stream(),
                    Some(format) => {
                        let format = &*format;
                        quote_spanned! {field.binding.span() =>
                            &#krate::core::value::FormattedValue::<_, #format, _>::new(#field)
                        }
                    }
                };
                self.writes.push(quote_spanned! {field.binding.span()=>
                    #krate::core::entry::EntryWriter::value(writer, #name, #field_tokens);
                });
                if sample_group.is_some() {
                    self.sample_groups
                        .push(quote_spanned! {field.binding.span()=>
                            ::std::iter::once((
                                ::std::borrow::Cow::Borrowed(#name),
                                #[allow(clippy::useless_conversion)]
                                {
                                    #krate::core::SampleGroup::as_sample_group(#field)
                                },
                            ))
                        });
                }
            }
            FieldMetricAttr::Ignore => {}
            FieldMetricAttr::Flatten => {
                self.writes.push(quote_spanned! {field.binding.span()=>
                    #krate::core::entry::Entry::write(#field, writer);
                });
                self.sample_groups
                    .push(quote_spanned! {field.binding.span()=>
                        #krate::core::entry::Entry::sample_group(#field)
                    });
            }
            FieldMetricAttr::Timestamp(span) => {
                if self.has_timestamp {
                    return Err(syn::Error::new(
                        span,
                        "can't have more than one `timestamp`",
                    ));
                } else {
                    self.has_timestamp = true;
                    // Note we have an explicit clippy allow so that if the timestamp is already a SystemTime, it
                    // doesn't generate code with a warning!
                    self.writes.push(quote_spanned! {field.binding.span()=>
                        #[allow(clippy::useless_conversion)]
                        {
                            #krate::core::entry::EntryWriter::timestamp(writer, (*#field).into());
                        }
                    });
                }
            }
        };
        Ok(())
    }
}

// Keeps track of what field names we've already seen to detect duplicates, plus any case renaming settings
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Namer {
    names: HashSet<String>,
    rename_all: Option<NameStyle>,
}

impl Namer {
    fn specified(&mut self, name: &SpannedValue<String>) -> syn::Result<String> {
        self.try_add(name.span(), name)
    }

    fn unspecified(&mut self, field: &BindingInfo<'_>) -> syn::Result<String> {
        let Some(ident) = field.ast().ident.as_ref() else {
            return Err(syn::Error::new(
                field.span(),
                "must specify `name` for tuple fields",
            ));
        };
        let name = ident.to_string();
        let name = self.rename_all.map(|r| r.apply(&name)).unwrap_or(name);
        self.try_add(ident.span(), &name)
    }

    fn try_add(&mut self, span: Span, name: &str) -> syn::Result<String> {
        if self.names.insert(name.into()) {
            Ok(name.into())
        } else {
            Err(syn::Error::new(
                span,
                format!("name `{name}` is used more than once"),
            ))
        }
    }
}

#[allow(clippy::enum_variant_names)] // "Case" is part of the name...
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameStyle {
    LowerCase,
    UpperCase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl NameStyle {
    fn try_parse(span: Span, style: &str) -> syn::Result<Self> {
        match style {
            "lowercase" => Ok(Self::LowerCase),
            "UPPERCASE" => Ok(Self::UpperCase),
            "PascalCase" => Ok(Self::PascalCase),
            "camelCase" => Ok(Self::CamelCase),
            "snake_case" => Ok(Self::SnakeCase),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebabCase),
            _ => Err(syn::Error::new(
                span,
                format!("unknown name style `{style}`"),
            )),
        }
    }

    fn apply(self, name: &str) -> String {
        use inflector::Inflector;
        match self {
            NameStyle::LowerCase => name.to_ascii_lowercase(),
            NameStyle::UpperCase => name.to_ascii_uppercase(),
            NameStyle::PascalCase => name.to_pascal_case(),
            NameStyle::CamelCase => name.to_camel_case(),
            NameStyle::SnakeCase => name.to_snake_case(),
            NameStyle::ScreamingSnakeCase => name.to_screaming_snake_case(),
            NameStyle::KebabCase => name.to_kebab_case(),
            NameStyle::ScreamingKebabCase => name.to_kebab_case().to_ascii_uppercase(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_tree_chain() {
        assert_eq!(
            make_binary_tree_chain(vec![]).to_string(),
            quote! {::std::iter::empty()}.to_string()
        );
        assert_eq!(
            make_binary_tree_chain(vec![quote! {1}]).to_string(),
            quote! {1}.to_string()
        );
        assert_eq!(
            make_binary_tree_chain(vec![quote! {1}, quote! {2}]).to_string(),
            quote! {1 .chain(2)}.to_string()
        );
        assert_eq!(make_binary_tree_chain(vec![quote!{1},quote!{2},quote!{3},quote!{4},quote!{5},quote!{6},quote!{7},quote!{8},quote!{9}]).to_string(),
            quote!{1 . chain (2) . chain (3 . chain (4)) . chain (5 . chain (6) . chain (7 . chain (8))) . chain (9)}.to_string());
        assert_eq!(make_binary_tree_chain(vec![quote!{1},quote!{2},quote!{3},quote!{4},quote!{5},quote!{6},quote!{7},quote!{8},quote!{9},quote!{10},quote!{11}]).to_string(),
            quote!{1 . chain (2) . chain (3 . chain (4)) . chain (5 . chain (6) . chain (7 . chain (8))) . chain (9 . chain (10) . chain (11))}.to_string());
    }

    #[test]
    fn derives_struct_entry() {
        synstructure::test_derive! {
            derive_entry {
                #[entry(rename_all = "PascalCase")]
                struct TestEntry {
                    #[entry(timestamp)]
                    start: SystemTime,
                    foo: String,
                    bar: String,
                    #[entry(sample_group)]
                    operation: &'static str,
                    #[entry(name = "GREAT_COUNTER")]
                    some_counter: u64,
                    #[entry(ignore)]
                    ignored: bool,
                    #[entry(flatten)]
                    sub_entry: SubEntry,
                    #[entry(format = my::Formatter)]
                    custom_format: bool,
                }
            }
            expands to {
                const _: () = {
                    impl ::metrique_writer::core::entry::Entry for TestEntry {
                        fn write<'a>(&'a self, writer: &mut impl ::metrique_writer::core::entry::EntryWriter<'a>) {
                            match *self {
                                TestEntry {
                                    start: ref __binding_0,
                                    foo: ref __binding_1,
                                    bar: ref __binding_2,
                                    operation: ref __binding_3,
                                    some_counter: ref __binding_4,
                                    ignored: ref __binding_5,
                                    sub_entry: ref __binding_6,
                                    custom_format: ref __binding_7,
                                } => {
                                    #[allow(clippy::useless_conversion)]
                                    {
                                        ::metrique_writer::core::entry::EntryWriter::timestamp(writer, (*__binding_0).into());
                                    }
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "Foo", __binding_1);
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "Bar", __binding_2);
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "Operation", __binding_3);
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "GREAT_COUNTER", __binding_4);
                                    ::metrique_writer::core::entry::Entry::write(__binding_6, writer);
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "CustomFormat",
                                        &::metrique_writer::core::value::FormattedValue::<_, my::Formatter, _>::new(__binding_7));
                                }
                            }
                        }

                        fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                            match *self {
                                TestEntry {
                                    start: ref __binding_0,
                                    foo: ref __binding_1,
                                    bar: ref __binding_2,
                                    operation: ref __binding_3,
                                    some_counter: ref __binding_4,
                                    ignored: ref __binding_5,
                                    sub_entry: ref __binding_6,
                                    custom_format: ref __binding_7,
                                } =>
                                    ::std::iter::once((
                                            ::std::borrow::Cow::Borrowed("Operation"),
                                            #[allow(clippy::useless_conversion)]
                                            {
                                                ::metrique_writer::core::SampleGroup::as_sample_group(__binding_3)
                                            },
                                    ))
                                    .chain(::metrique_writer::core::entry::Entry::sample_group(__binding_6)),
                            }
                        }
                    }
                };
            }
            no_build
        }
    }

    #[test]
    fn derives_struct_entry_metrique() {
        synstructure::test_derive! {
            derive_metrique_entry {
                #[entry(rename_all = "PascalCase")]
                struct TestEntry {
                    #[entry(timestamp)]
                    start: SystemTime,
                }
            }
            expands to {
                const _: () = {
                    impl ::metrique::writer::core::entry::Entry for TestEntry {
                        fn write<'a>(&'a self, writer: &mut impl ::metrique::writer::core::entry::EntryWriter<'a>) {
                            match *self {
                                TestEntry {
                                    start: ref __binding_0,
                                } => {
                                    #[allow(clippy::useless_conversion)]
                                    {
                                        ::metrique::writer::core::entry::EntryWriter::timestamp(writer, (*__binding_0).into());
                                    }
                                }
                            }
                        }

                        fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                            match *self {
                                TestEntry {
                                    start: ref __binding_0,
                                } => ::std::iter::empty(),
                            }
                        }
                    }
                };
            }
            no_build
        }
    }

    #[test]
    fn checks_duplicate_names() {
        synstructure::test_derive! {
            derive_entry {
                struct TestEntry {
                    first: String,
                    #[entry(name = "first")]
                    second: String
                }
            }
            expands to {
                ::core::compile_error! { "name `first` is used more than once" }
            }
            no_build
        }
    }

    #[test]
    fn checks_duplicate_timestamps() {
        synstructure::test_derive! {
            derive_entry {
                struct TestEntry {
                    #[entry(timestamp)]
                    first: String,
                    #[entry(timestamp)]
                    second: String
                }
            }
            expands to {
                ::core::compile_error! { "can't have more than one `timestamp`" }
            }
            no_build
        }
    }

    #[test]
    fn derives_enum_entry() {
        synstructure::test_derive! {
            derive_entry {
                #[entry(rename_all = "PascalCase")]
                enum TestEntry {
                    First(#[entry(flatten)] FirstEntry),
                    Second {
                        #[entry(sample_group)]
                        test: &'static str,
                        #[entry(timestamp)]
                        time: SystemTime,
                        some_counter: u64,
                    },
                    Third(#[entry(name = "CanNameTuples")] u64)
                }
            }
            expands to {
                const _: () = {
                    impl ::metrique_writer::core::entry::Entry for TestEntry {
                        fn write<'a>(&'a self, writer: &mut impl ::metrique_writer::core::entry::EntryWriter<'a>) {
                            match *self {
                                TestEntry::First(ref __binding_0,) => {
                                    ::metrique_writer::core::entry::Entry::write(__binding_0, writer);
                                }
                                TestEntry::Second { test: ref __binding_0, time: ref __binding_1, some_counter: ref __binding_2, } => {
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "Test", __binding_0);
                                    #[allow(clippy::useless_conversion)]
                                    {
                                        ::metrique_writer::core::entry::EntryWriter::timestamp(writer, (*__binding_1).into());
                                    }
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "SomeCounter", __binding_2);
                                }
                                TestEntry::Third(ref __binding_0,) => {
                                    ::metrique_writer::core::entry::EntryWriter::value(writer, "CanNameTuples", __binding_0);
                                }
                            }
                        }

                        fn sample_group(&self) -> impl ::std::iter::Iterator<Item = (::std::borrow::Cow<'static, str>, ::std::borrow::Cow<'static, str>)> {
                            match *self {
                                TestEntry::First(ref __binding_0,) =>
                                    Box::new(
                                        ::metrique_writer::core::entry::Entry::sample_group(__binding_0)
                                    ) as Box<dyn ::std::iter::Iterator<Item = _>>,
                                TestEntry::Second { test: ref __binding_0, time: ref __binding_1, some_counter: ref __binding_2, } =>
                                    Box::new(
                                        ::std::iter::once((
                                            ::std::borrow::Cow::Borrowed("Test"),
                                            #[allow(clippy::useless_conversion)]
                                            {
                                                ::metrique_writer::core::SampleGroup::as_sample_group(__binding_0)
                                            },
                                        ))
                                    ) as Box<dyn ::std::iter::Iterator<Item = _>>,
                                TestEntry::Third(ref __binding_0,) =>
                                    Box::new(
                                        ::std::iter::empty()
                                    ) as Box<dyn ::std::iter::Iterator<Item = _>>,
                            }
                        }
                    }
                };
            }
            no_build
        }
    }
}
