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
