// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

mod inner {
    use metrique::unit_of_work::metrics;

    #[metrics]
    pub struct PublicMetricsInMod {
        pub public_field: usize,
        private_field: usize,
        pub(crate) crate_visible: usize,
    }

    pub fn create_and_export() -> PublicMetricsInMod {
        PublicMetricsInMod {
            public_field: 10,
            private_field: 20,
            crate_visible: 30,
        }
    }
}

fn main() {
    // Create metrics from the inner module
    let metrics_from_mod = inner::create_and_export();

    // This should compile fine - public field
    let _val1 = metrics_from_mod.public_field;

    // This should fail - private field
    let _val2 = metrics_from_mod.private_field;
}
