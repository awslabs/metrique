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

#[cfg(test)]
mod tests {
    use metrique::CloseValue;
    use metrique::unit_of_work::metrics;
    use metrique_core::DynamicNameStyle;
    use assert2::check;
    use metrique_writer::test_util::to_test_entry;
    use rstest::rstest;

    use super::DynamicInflectionEntry;

    #[metrics(subfield_owned)]
    #[derive(Default)]
    struct Inner {
        count: u64,
    }

    #[metrics]
    #[derive(Default)]
    struct Outer {
        #[metrics(flatten, prefix = "pfx_")]
        inner: Inner,
        other: u64,
    }

    #[rstest]
    #[case::identity(DynamicNameStyle::Identity, "pfx_count", "other")]
    #[case::pascal_case(DynamicNameStyle::PascalCase, "PfxCount", "Other")]
    #[case::snake_case(DynamicNameStyle::SnakeCase, "pfx_count", "other")]
    #[case::kebab_case(DynamicNameStyle::KebabCase, "pfx-count", "other")]
    fn prefix_inflection(
        #[case] style: DynamicNameStyle,
        #[case] expected_prefixed: &str,
        #[case] expected_plain: &str,
    ) {
        let entry = DynamicInflectionEntry {
            entry: Outer {
                inner: Inner { count: 42 },
                other: 7,
            }
            .close(),
            name_style: style,
        };
        let t = to_test_entry(entry);
        check!(t.metrics[expected_prefixed] == 42);
        check!(t.metrics[expected_plain] == 7);
    }

    #[metrics]
    struct WithRuntimeMetrics {
        #[metrics(flatten, prefix = "rt_")]
        runtime: tokio_metrics::RuntimeMetrics,
        request_count: u64,
    }

    #[rstest]
    #[case::identity(
        DynamicNameStyle::Identity,
        "request_count",
        "rt_workers_count",
        "rt_total_park_count"
    )]
    #[case::pascal_case(
        DynamicNameStyle::PascalCase,
        "RequestCount",
        "RtWorkersCount",
        "RtTotalParkCount"
    )]
    fn runtime_metrics_with_prefix(
        #[case] style: DynamicNameStyle,
        #[case] request_key: &str,
        #[case] workers_key: &str,
        #[case] park_key: &str,
    ) {
        let entry = DynamicInflectionEntry {
            entry: WithRuntimeMetrics {
                runtime: tokio_metrics::RuntimeMetrics::default(),
                request_count: 5,
            }
            .close(),
            name_style: style,
        };
        let t = to_test_entry(entry);
        check!(t.metrics[request_key] == 5);
        check!(t.metrics[workers_key] == 0);
        check!(t.metrics[park_key].as_u64() == 0);
    }
}
