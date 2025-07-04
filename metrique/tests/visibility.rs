// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

// Test that the macro preserves visibility modifiers on structs
#[metrics(rename_all = "PascalCase")]
pub struct PublicMetrics {
    pub public_field: usize,
    private_field: usize,
    pub(crate) crate_visible: usize,
}

#[metrics(rename_all = "PascalCase")]
struct PrivateMetrics {
    pub public_field: usize,
    private_field: usize,
    pub(crate) crate_visible: usize,
}

#[metrics(rename_all = "PascalCase")]
pub(crate) struct CrateVisibleMetrics {
    pub public_field: usize,
    private_field: usize,
    pub(crate) crate_visible: usize,
}

mod inner {
    use metrique::unit_of_work::metrics;

    #[metrics(rename_all = "PascalCase")]
    pub struct PublicMetricsInMod {
        pub public_field: usize,
        private_field: usize,
        pub(crate) crate_visible: usize,
        pub(super) super_visible: usize,
    }

    #[metrics(rename_all = "PascalCase")]
    pub(super) struct SuperVisibleMetrics {
        pub(super) field: usize,
        pub(super) private_field: usize,
    }

    pub fn create_and_export() -> PublicMetricsInMod {
        PublicMetricsInMod {
            public_field: 10,
            private_field: 20,
            crate_visible: 30,
            super_visible: 40,
        }
    }
}

fn main() {
    // Test public struct with mixed visibility fields
    let _public_metrics = PublicMetrics {
        public_field: 1,
        private_field: 2,
        crate_visible: 3,
    };

    // Test private struct with mixed visibility fields
    let _private_metrics = PrivateMetrics {
        public_field: 1,
        private_field: 2,
        crate_visible: 3,
    };

    // Test crate-visible struct with mixed visibility fields
    let _crate_metrics = CrateVisibleMetrics {
        public_field: 1,
        private_field: 2,
        crate_visible: 3,
    };

    // Test accessing a public struct from another module
    let metrics_from_mod = inner::create_and_export();

    // Should be able to access public field
    let _val = metrics_from_mod.public_field;

    // Should be able to access super-visible field
    let _val = metrics_from_mod.super_visible;

    // Test creating a super-visible struct directly
    let super_metrics = inner::SuperVisibleMetrics {
        field: 100,
        private_field: 200,
    };

    // Should be able to access public field
    let _val = super_metrics.field;
}
