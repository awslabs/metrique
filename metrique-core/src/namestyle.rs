// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains various name styles

use std::marker::PhantomData;

use crate::concat::{Concatenated, EmptyConstStr, MaybeConstStr};

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

    #[doc(hidden)]
    type AppendPrefix<T: MaybeConstStr>: NameStyle;

    /// Inflect the name, adding prefixes
    #[doc(hidden)]
    type Inflect<ID: MaybeConstStr, PASCAL: MaybeConstStr, SNAKE: MaybeConstStr, KEBAB: MaybeConstStr>: MaybeConstStr;

    /// Inflect an affix (just inflect, without adding prefixes)
    #[doc(hidden)]
    type InflectAffix<ID: MaybeConstStr, PASCAL: MaybeConstStr, SNAKE: MaybeConstStr, KEBAB: MaybeConstStr>: MaybeConstStr;
}

// Style index constants used by descriptor codegen to select the right static.
// The macro crate mirrors these in `metrique-macro/src/inflect.rs` (DESCRIPTOR_STYLES
// and DESCRIPTOR_STYLE_NAMES). Both must stay in sync.
#[doc(hidden)]
pub const STYLE_PRESERVE: u8 = 0;
#[doc(hidden)]
pub const STYLE_PASCAL: u8 = 1;
#[doc(hidden)]
pub const STYLE_SNAKE: u8 = 2;
#[doc(hidden)]
pub const STYLE_KEBAB: u8 = 3;

// Compile-time assertion: style constants must be sequential starting from 0.
// If you add or reorder styles, update metrique-macro/src/inflect.rs
// (DESCRIPTOR_STYLES, DESCRIPTOR_STYLE_NAMES, descriptor_index) to match.
const _: () = {
    assert!(STYLE_PRESERVE == 0);
    assert!(STYLE_PASCAL == 1);
    assert!(STYLE_SNAKE == 2);
    assert!(STYLE_KEBAB == 3);
};

/// Inflects names to the identity case
pub struct Identity<PREFIX: MaybeConstStr = EmptyConstStr>(PhantomData<PREFIX>);
impl<PREFIX: MaybeConstStr> private::NameStyleInternal for Identity<PREFIX> {}
impl<PREFIX: MaybeConstStr> NameStyle for Identity<PREFIX> {
    type KebabCase = KebabCase<PREFIX>;
    type PascalCase = PascalCase<PREFIX>;
    type SnakeCase = SnakeCase<PREFIX>;
    type AppendPrefix<P: MaybeConstStr> = Identity<Concatenated<PREFIX, P>>;
    type Inflect<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = Concatenated<PREFIX, ID>;
    type InflectAffix<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = ID;
}

/// inflects names to `PascalCase`
pub struct PascalCase<PREFIX: MaybeConstStr = EmptyConstStr>(PhantomData<PREFIX>);
impl<PREFIX: MaybeConstStr> private::NameStyleInternal for PascalCase<PREFIX> {}
impl<PREFIX: MaybeConstStr> NameStyle for PascalCase<PREFIX> {
    type KebabCase = KebabCase<PREFIX>;
    type PascalCase = PascalCase<PREFIX>;
    type SnakeCase = SnakeCase<PREFIX>;
    type AppendPrefix<P: MaybeConstStr> = PascalCase<Concatenated<PREFIX, P>>;
    type Inflect<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = Concatenated<PREFIX, PASCAL>;
    type InflectAffix<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = PASCAL;
}

/// Inflects names to `snake_case`
pub struct SnakeCase<PREFIX: MaybeConstStr = EmptyConstStr>(PhantomData<PREFIX>);
impl<PREFIX: MaybeConstStr> private::NameStyleInternal for SnakeCase<PREFIX> {}
impl<PREFIX: MaybeConstStr> NameStyle for SnakeCase<PREFIX> {
    type KebabCase = KebabCase<PREFIX>;
    type PascalCase = PascalCase<PREFIX>;
    type SnakeCase = SnakeCase<PREFIX>;
    type AppendPrefix<P: MaybeConstStr> = SnakeCase<Concatenated<PREFIX, P>>;
    type Inflect<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = Concatenated<PREFIX, SNAKE>;
    type InflectAffix<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = SNAKE;
}

/// Inflects names to `kebab-case`
pub struct KebabCase<PREFIX: MaybeConstStr = EmptyConstStr>(PhantomData<PREFIX>);
impl<PREFIX: MaybeConstStr> private::NameStyleInternal for KebabCase<PREFIX> {}
impl<PREFIX: MaybeConstStr> NameStyle for KebabCase<PREFIX> {
    type KebabCase = KebabCase<PREFIX>;
    type PascalCase = PascalCase<PREFIX>;
    type SnakeCase = SnakeCase<PREFIX>;
    type AppendPrefix<P: MaybeConstStr> = KebabCase<Concatenated<PREFIX, P>>;
    type Inflect<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = Concatenated<PREFIX, KEBAB>;
    type InflectAffix<
        ID: MaybeConstStr,
        PASCAL: MaybeConstStr,
        SNAKE: MaybeConstStr,
        KEBAB: MaybeConstStr,
    > = KEBAB;
}

/// Runtime-selectable name style for metric field names.
///
/// This mirrors the compile-time [`NameStyle`] types (`Identity`, `PascalCase`,
/// etc.) as enum variants for use in runtime configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DynamicNameStyle {
    /// Keep original field names.
    #[default]
    Identity,
    /// Convert to PascalCase (e.g. `WorkersCount`).
    PascalCase,
    /// Convert to snake_case (e.g. `workers_count`).
    SnakeCase,
    /// Convert to kebab-case (e.g. `workers-count`).
    KebabCase,
}
