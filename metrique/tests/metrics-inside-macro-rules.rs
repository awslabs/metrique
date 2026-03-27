// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Tests for `#[metrics]` on types declared inside `macro_rules!`.
//!
//! This mirrors tokio-metrics style declarations where:
//! - field identifiers come from macro parameters (invocation-site hygiene), and
//! - some fields are gated or modified through `cfg` / `cfg_attr`.

use metrique::writer::test_util;
use metrique::{CloseValue, RootEntry};
use metrique_macro::metrics;

macro_rules! define_runtime_metrics_like_tokio {
    (
        stable {
            $(
                $(#[$($attributes:tt)*])*
                $vis:vis $name:ident : $ty:ty
            ),* $(,)?
        }
        unstable {
            $(
                $(#[$($unstable_attributes:tt)*])*
                $unstable_vis:vis $unstable_name:ident : $unstable_ty:ty
            ),* $(,)?
        }
    ) => {
        #[cfg_attr(test, metrics(subfield, rename_all = "PascalCase"))]
        #[derive(Default, Clone)]
        #[allow(dead_code)]
        struct RuntimeLikeMetrics {
            $(
                $(#[$($attributes)*])*
                $vis $name: $ty,
            )*
            $(
                $(#[$($unstable_attributes)*])*
                #[cfg(test)] // stand-in for something like #[cfg(tokio_unstable)]
                $unstable_vis $unstable_name: $unstable_ty,
            )*
        }
    };
}

define_runtime_metrics_like_tokio! {
    stable {
        pub workers_count: usize,
        pub busy_duration: u64,
    }
    unstable {
        #[metrics(ignore)]
        pub poll_time_histogram: Vec<u64>,
        pub steal_count: u64,
    }
}

#[test]
fn macro_rules_declared_fields_emit_metrics() {
    let m = RuntimeLikeMetrics {
        workers_count: 10,
        busy_duration: 500,
        steal_count: 42,
        poll_time_histogram: vec![1, 2, 3],
    };

    let entry = test_util::to_test_entry(RootEntry::new(m.close()));
    assert_eq!(entry.metrics["WorkersCount"], 10);
    assert_eq!(entry.metrics["BusyDuration"], 500);
    assert_eq!(entry.metrics["StealCount"], 42);
    assert!(
        !entry.metrics.contains_key("PollTimeHistogram"),
        "field with #[metrics(ignore)] should not appear in {entry:?}"
    );
}

/// Tests that cfg-disabled fields are not referenced by generated code.
macro_rules! define_compile_time_cfg_metrics {
    ( $( $field:ident : $ty:ty ),* $(,)? ) => {
        #[metrics(subfield, rename_all = "PascalCase")]
        #[derive(Default, Clone)]
        struct CompileTimeCfgMetrics {
            $( $field: $ty, )*
            #[cfg(doc)]
            phantom_field: u64,
            #[cfg_attr(test, cfg(doc))]
            #[metrics(sample_group)]
            cfg_attr_sample_group: &'static str,
        }
    };
}

define_compile_time_cfg_metrics! {
    real_field: u64,
}

#[test]
fn cfg_disabled_fields_are_not_referenced() {
    // `#[cfg(doc)]` is false in this test target, so `phantom_field` does not exist.
    // Generated write/close/sample_group code must therefore compile without touching it.
    //
    // `cfg_attr_sample_group` is similarly removed by `#[cfg_attr(test, cfg(doc))]` and
    // still must not be referenced by generated sample-group code.
    let m = CompileTimeCfgMetrics { real_field: 5 };
    let entry = test_util::to_test_entry(RootEntry::new(m.close()));
    assert_eq!(entry.metrics["RealField"], 5);
    assert!(
        !entry.metrics.contains_key("PhantomField"),
        "cfg-excluded field should not appear in {entry:?}"
    );
    assert!(
        !entry.metrics.contains_key("CfgAttrSampleGroup"),
        "cfg_attr(cfg(...)) excluded field should not appear in {entry:?}"
    );
}
