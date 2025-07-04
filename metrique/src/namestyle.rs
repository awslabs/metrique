// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub trait SetNamestyle {
    type Output<Style>;
    fn set_style<Style>(self) -> Self::Output<Style>;
}

impl<T> SetNamestyle for Option<T>
where
    T: SetNamestyle,
{
    type Output<Style> = Option<T::Output<Style>>;

    fn set_style<Style>(self) -> Self::Output<Style> {
        self.map(|v| v.set_style())
    }
}

pub struct Identity;
pub struct PascalCase;
pub struct SnakeCase;
pub struct KebabCase;
