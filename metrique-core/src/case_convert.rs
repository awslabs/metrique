// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Case-conversion primitives shared by the metrique proc-macro crates.
//!
//! These wrap the [`inflector`](https://crates.io/crates/str_inflector) crate so
//! the dependency lives in a single place. The `#[metrics]` and `derive(Entry)`
//! macros parse their own attribute syntax and map the result onto [`CaseStyle`],
//! then call [`CaseStyle::apply`] (or [`CaseStyle::apply_prefix`]) to perform the
//! conversion.

use inflector::Inflector;

/// A case-conversion style applied to metric field and variant names.
///
/// This is the union of the styles accepted by the `#[metrics]` and
/// `derive(Entry)` macros. Each macro maps its own parsed attribute onto this
/// enum, so not every variant is reachable from every macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaseStyle {
    /// Keep the original name unchanged.
    Preserve,
    /// `lowercase`
    Lower,
    /// `UPPERCASE`
    Upper,
    /// `PascalCase`
    Pascal,
    /// `camelCase`
    Camel,
    /// `snake_case`
    Snake,
    /// `SCREAMING_SNAKE_CASE`
    ScreamingSnake,
    /// `kebab-case`
    Kebab,
    /// `SCREAMING-KEBAB-CASE`
    ScreamingKebab,
}

impl CaseStyle {
    /// Convert `name` to this case style.
    pub fn apply(self, name: &str) -> String {
        match self {
            CaseStyle::Preserve => name.to_string(),
            CaseStyle::Lower => name.to_ascii_lowercase(),
            CaseStyle::Upper => name.to_ascii_uppercase(),
            CaseStyle::Pascal => name.to_pascal_case(),
            CaseStyle::Camel => name.to_camel_case(),
            CaseStyle::Snake => name.to_snake_case(),
            CaseStyle::ScreamingSnake => name.to_screaming_snake_case(),
            CaseStyle::Kebab => name.to_kebab_case(),
            CaseStyle::ScreamingKebab => name.to_kebab_case().to_ascii_uppercase(),
        }
    }

    /// Convert `name` to this case style for use as a prefix, ensuring the
    /// result ends with the style's delimiter so it can be concatenated with a
    /// following name.
    ///
    /// Styles without a delimiter (`PascalCase`, `camelCase`, `lowercase`,
    /// `UPPERCASE`) and `Preserve` are returned unchanged from [`apply`].
    ///
    /// [`apply`]: CaseStyle::apply
    pub fn apply_prefix(self, name: &str) -> String {
        match self {
            CaseStyle::Snake | CaseStyle::ScreamingSnake => {
                let mut res = self.apply(name);
                if !res.ends_with('_') {
                    res.push('_');
                }
                res
            }
            CaseStyle::Kebab | CaseStyle::ScreamingKebab => {
                let mut res = self.apply(name);
                if !res.ends_with('-') {
                    res.push('-');
                }
                res
            }
            CaseStyle::Preserve
            | CaseStyle::Lower
            | CaseStyle::Upper
            | CaseStyle::Pascal
            | CaseStyle::Camel => self.apply(name),
        }
    }
}

/// Returns the first character in `name` that cannot be inflected (anything
/// other than an alphanumeric, `_`, or `-`), if any.
pub fn name_contains_uninflectables(name: &str) -> Option<char> {
    name.chars()
        .find(|&c| !c.is_alphanumeric() && c != '_' && c != '-')
}

/// Returns `true` if `name` ends with a `_` or `-` delimiter.
pub fn name_ends_with_delimiter(name: &str) -> bool {
    let last = name.chars().last();
    last == Some('_') || last == Some('-')
}

// `.` is currently used in production, make it a warning instead of an error
/// Returns `true` if `name` contains a `.`.
pub fn name_contains_dot(name: &str) -> bool {
    name.contains('.')
}
