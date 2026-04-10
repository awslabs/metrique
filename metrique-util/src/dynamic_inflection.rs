// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::{AtomicBool, Ordering};

use metrique::InflectableEntry;
use metrique::writer::{Entry, EntryWriter};
use metrique_core::{DynamicNameStyle, Identity, KebabCase, PascalCase, SnakeCase};

use crate::MetricNameStyle;

/// Adapter that bridges [`InflectableEntry`] to [`Entry`] by selecting the
/// field name inflection ([`MetricNameStyle`]) at runtime.
///
/// Use this when the name style is determined by configuration rather than
/// at compile time.
pub(crate) struct DynamicInflectionEntry<M> {
    pub(crate) entry: M,
    pub(crate) name_style: MetricNameStyle,
}

impl<M> Entry for DynamicInflectionEntry<M>
where
    M: InflectableEntry<Identity>
        + InflectableEntry<PascalCase>
        + InflectableEntry<SnakeCase>
        + InflectableEntry<KebabCase>,
{
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        match self.name_style {
            DynamicNameStyle::Identity => InflectableEntry::<Identity>::write(&self.entry, w),
            DynamicNameStyle::PascalCase => InflectableEntry::<PascalCase>::write(&self.entry, w),
            DynamicNameStyle::SnakeCase => InflectableEntry::<SnakeCase>::write(&self.entry, w),
            DynamicNameStyle::KebabCase => InflectableEntry::<KebabCase>::write(&self.entry, w),
            _ => {
                static WARNED_UNKNOWN_NAME_STYLE: AtomicBool = AtomicBool::new(false);
                if !WARNED_UNKNOWN_NAME_STYLE.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        ?self.name_style,
                        "unknown MetricNameStyle variant; falling back to Identity"
                    );
                }
                InflectableEntry::<Identity>::write(&self.entry, w)
            }
        }
    }
}
