// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::ops::{Deref, DerefMut};

use metrique_writer_core::{Entry, EntryWriter, Value};

/// Wrapper type for [`enum_map`](https://crates.io/crates/enum-map)-style maps of enum keys to optional [`Value`]s.
///
/// Enum maps offer an efficient way to represent a set of related metrics or flags. They can be queried and written by
/// key in O(1) without hashing. For example, a set of optional flag values could be modeled as
/// ```
/// # use enum_map::{Enum, EnumMap};
/// # use strum_macros::IntoStaticStr;
/// # use metrique_writer::{Entry, entry::EnumMapEntry};
/// #[derive(Enum, IntoStaticStr, Clone, Copy)]
/// pub enum ErrorFlag {
///     CloseConnection,
///     DownstreamReadFailure,
///     DownstreamConnectionClose,
///     MissingUpstreamConnection,
///     Panic,
/// }
///
/// #[derive(Entry, Default)]
/// struct RequestMetrics {
///     #[entry(flatten)]
///     error_flags: EnumMapEntry<EnumMap<ErrorFlag, Option<bool>>>,
///     // ...
/// }
///
/// let mut request_metrics = RequestMetrics::default();
/// request_metrics.error_flags[ErrorFlag::Panic] = Some(true);
/// ```
///
/// The inner type must be iterable by reference, yielding (key, value) pairs. The enum key must impl
/// `Into<&'static str>`. This can be derived automatically using the
/// [strum_macros](https://crates.io/crates/strum_macros) crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct EnumMapEntry<E>(E);

impl<E, K, V> Entry for EnumMapEntry<E>
where
    for<'a> &'a E: IntoIterator<Item = (K, &'a V)>,
    K: Into<&'static str>,
    V: Value,
{
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        for (key, value) in &self.0 {
            writer.value(key.into(), value);
        }
    }
}

impl<E> EnumMapEntry<E> {
    pub fn new(map: E) -> Self {
        Self(map)
    }

    pub fn into_inner(self) -> E {
        self.0
    }
}

impl<E> From<E> for EnumMapEntry<E> {
    fn from(value: E) -> Self {
        Self::new(value)
    }
}

impl<E> Deref for EnumMapEntry<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<E> DerefMut for EnumMapEntry<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
