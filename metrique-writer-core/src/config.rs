// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains configurations that are mostly useful for EMF-format metrics
//!
//! The configurations are in this crate in the interest of interoperability

use std::{borrow::Cow, slice};

use crate::EntryConfig;

/// This config enables splitting entries in case of multiple dimension values.
///
/// Mostly useful for EMF, but your own custom formatters can use it too.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AllowSplitEntries(());

impl AllowSplitEntries {
    /// Create a new [AllowSplitEntries]
    pub const fn new() -> Self {
        Self(())
    }
}

impl EntryConfig for AllowSplitEntries {}

/// This config is used for basic error messages. It allows generating
/// error messages that will not be routed properly (for example,
/// EMF errors missing dimensions) so that something
/// will get out even if globals are misconfigured.
///
/// This should still not allow
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct IsBasicErrorMessage(());

impl IsBasicErrorMessage {
    /// Create a new [IsBasicErrorMessage]
    const fn new() -> Self {
        Self(())
    }
}

impl EntryConfig for IsBasicErrorMessage {}

/// An entry that represents a basic error message that can be used to
/// get a log message to the metrics stream even if globals are
/// misconfigured.
pub struct BasicErrorMessage<'a> {
    message: &'a str,
}

impl<'a> BasicErrorMessage<'a> {
    /// Create a new [BasicErrorMessage].
    pub fn new(message: &'a str) -> Self {
        Self { message }
    }
}

#[diagnostic::do_not_recommend]
impl crate::Entry for BasicErrorMessage<'_> {
    fn write<'a>(&'a self, writer: &mut impl crate::EntryWriter<'a>) {
        writer.config(&const { IsBasicErrorMessage::new() });
        writer.value("Error", self.message);
    }
}

/// This struct is mostly useful for the EMF internal implementation
pub struct DimensionsIterator<'a> {
    inner: slice::Iter<'a, Cow<'static, str>>,
}

impl<'a> Iterator for DimensionsIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|d| &**d)
    }
}

/// Putting this config on an entry will make supporting formatters extend
/// their dimension-sets with the specified dimensions. Currently, this is supported
/// by the EMF formatter by cartesian-producting the dimensions in this struct
/// with its configured ("dev-ops") dimensions.
///
/// This is useful when some of your [`Entry`] members have different dimensions than others.
///
/// ## Example
///
/// ```
/// # use std::borrow::Cow;
/// # use metrique_writer_core::config::EntryDimensions;
/// # use metrique_writer_core::{Entry, EntryWriter};
///
/// struct MyEntry;
/// impl Entry for MyEntry {
///     fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
///         writer.value("AWSAccountId", "012345678901");
///         writer.value("API", "MyAPI");
///         writer.value("StringProp", "some string value");
///         writer.value("SomeField", &2u32);
///         writer.config(
///             const {
///                 &EntryDimensions::new(Cow::Borrowed(&[
///                     Cow::Borrowed(&[Cow::Borrowed("API")]),
///                     Cow::Borrowed(&[Cow::Borrowed("API"), Cow::Borrowed("StringProp")]),
///                 ]))
///             },
///         );
///         // ...
///     }
/// }
/// ```
///
/// Note that if you are using the `metrique` library, you can also get a similar effect
/// with the `#[metrics]` proc macro, at least for the `EntryDimensions` config:
///
/// ```
/// use metrique::unit_of_work::metrics;
///
/// #[metrics(emf::dimension_sets = [[], ["AWSAccountId"]])]
/// struct MyEntry {
///     #[metrics(name = "AWSAccountId")]
///     aws_account_id: String,
///     #[metrics(name = "API")]
///     api: String,
///     string_prop: String,
///     some_field: u32,
/// }
/// ```
///
/// Assuming your `Emf` was created as follows with configured ("dev-ops")
/// dimensions `[[], ["AWSAccountID"]]`:
///
/// ```
/// # use metrique_writer_format_emf::Emf;
/// Emf::all_validations("MyNS".to_string(), vec![vec![], vec!["AWSAccountId".to_string()]])
/// # ;
/// ```
///
/// Then, both when implementing [`Entry`] directly or when using `metrique`,
/// the emitted metric will be emitted under these 4 dimension sets:
///
/// ```notrust
/// ["API"],
/// ["API", "StringProp"],
/// ["AWSAccountId", "API"],
/// ["AWSAccountId", "API", "StringProp"],
/// ```
///
/// [`Entry`]: crate::Entry
#[derive(Clone, Debug)]
pub struct EntryDimensions {
    dimensions: Cow<'static, [Cow<'static, [Cow<'static, str>]>]>,
}

impl EntryDimensions {
    /// Create a new [EntryDimensions]
    pub const fn new(dimensions: Cow<'static, [Cow<'static, [Cow<'static, str>]>]>) -> Self {
        Self { dimensions }
    }

    /// A wrapper around [`EntryDimensions::new`] that avoids one layer of Cow.
    pub const fn new_static(dimensions: &'static [Cow<'static, [Cow<'static, str>]>]) -> Self {
        Self {
            dimensions: Cow::Borrowed(dimensions),
        }
    }

    /// This method is mostly useful for the EMF internal implementation
    pub fn is_empty(&self) -> bool {
        self.dimensions.is_empty()
    }

    /// This method is mostly useful for the EMF internal implementation
    ///
    /// Intentionally abstracts over the EntryDimensions implementation details to reduce breakage risk
    pub fn dim_sets<'a>(&'a self) -> impl Iterator<Item = DimensionsIterator<'a>> {
        self.dimensions.iter().map(|inner| DimensionsIterator {
            inner: inner.iter(),
        })
    }
}

impl EntryConfig for EntryDimensions {}

#[cfg(test)]
mod test {
    use super::EntryDimensions;
    use std::borrow::Cow;

    #[test]
    fn test_misc_coverage() {
        // coverage doesn't count const fns. Get it to avoid annoying me.
        assert!(
            EntryDimensions::new(Cow::Borrowed(&[Cow::Borrowed(&[])])).dimensions[0].is_empty()
        );
        assert!(EntryDimensions::new_static(&[Cow::Borrowed(&[])]).dimensions[0].is_empty());
    }
}
