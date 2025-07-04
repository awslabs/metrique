// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains various name styles

use std::borrow::Cow;

pub(crate) mod private {
    /// Helper trait to make `NameStyle` sealed
    pub trait NameStyleInternal {}
}

/// This trait is used to describe name styles for [`InflectableEntry`].
///
/// The exact implementation of this trait is currently unstable.
///
/// [`InflectableEntry`]: crate::InflectableEntry
pub trait NameStyle: private::NameStyleInternal {
    #[doc(hidden)]
    type KebabCase: NameStyle;

    #[doc(hidden)]
    type PascalCase: NameStyle;

    #[doc(hidden)]
    type SnakeCase: NameStyle;

    /// In theory this function does not pose a back compat hazard, but it
    /// is still better if people only call it via the macro
    #[doc(hidden)]
    fn inflect_name(
        identity: &'static str,
        pascal: &'static str,
        snake: &'static str,
        kebab: &'static str,
    ) -> Cow<'static, str>;
}

/// Inflects names to the identity case
pub struct Identity;
impl private::NameStyleInternal for Identity {}
impl NameStyle for Identity {
    type KebabCase = KebabCase;
    type PascalCase = PascalCase;
    type SnakeCase = SnakeCase;

    #[inline]
    fn inflect_name(
        identity: &'static str,
        _pascal: &'static str,
        _snake: &'static str,
        _kebab: &'static str,
    ) -> Cow<'static, str> {
        Cow::Borrowed(identity)
    }
}

/// inflects names to `PascalCase`
pub struct PascalCase;

impl private::NameStyleInternal for PascalCase {}
impl NameStyle for PascalCase {
    type KebabCase = KebabCase;
    type PascalCase = PascalCase;
    type SnakeCase = SnakeCase;

    #[inline]
    fn inflect_name(
        _identity: &'static str,
        pascal: &'static str,
        _snake: &'static str,
        _kebab: &'static str,
    ) -> Cow<'static, str> {
        Cow::Borrowed(pascal)
    }
}

/// Inflects names to `snake_case`
pub struct SnakeCase;
impl private::NameStyleInternal for SnakeCase {}
impl NameStyle for SnakeCase {
    type KebabCase = KebabCase;
    type PascalCase = PascalCase;
    type SnakeCase = SnakeCase;

    #[inline]
    fn inflect_name(
        _identity: &'static str,
        _pascal: &'static str,
        snake: &'static str,
        _kebab: &'static str,
    ) -> Cow<'static, str> {
        Cow::Borrowed(snake)
    }
}

/// Inflects names to `kebab-case`
pub struct KebabCase;
impl private::NameStyleInternal for KebabCase {}
impl NameStyle for KebabCase {
    type KebabCase = KebabCase;
    type PascalCase = PascalCase;
    type SnakeCase = SnakeCase;

    #[inline]
    fn inflect_name(
        _identity: &'static str,
        _pascal: &'static str,
        _snake: &'static str,
        kebab: &'static str,
    ) -> Cow<'static, str> {
        Cow::Borrowed(kebab)
    }
}
