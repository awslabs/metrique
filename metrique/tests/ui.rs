// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

// The UI tests need to be specific to a rustc version, since error output
// changes between versions.
//
// WARNING: This should be testing a version that is explicitly tested in our CI
// rather than merely being the latest stable, to ensure the test does not "vanish"
// when Rust releases a new version. So, MAKE SURE THAT THIS RUSTC VERSION IS IN SYNC
// WITH AN EXPLICIT RUSTC VERSION IN `.github/workflows/build.yml` (TYPICALLY THE
// MSRV), OTHERWISE THE UI TESTS WILL BE SKIPPED IN CI.
#[rustversion::attr(not(stable(1.87.0)), deprecated(note = "Update the rustc version (you may need to update UI tests)"))]
#[allow(dead_code)]
fn matching_rust_version() -> bool {
    rustversion::cfg!(stable(1.87.0))
}

#[test]
fn test_metrics_macro() {
    if !matching_rust_version() {
        eprintln!(
            "Rust version does not match. not running tests to avoid spurious failures. Update the Rust version to match the current rust version"
        );
        return;
    }
    let t = trybuild::TestCases::new();
    // Test cases that should compile successfully
    t.pass("tests/ui/pass/*.rs");
    // Test cases that should fail with specific errors
    t.compile_fail("tests/ui/fail/*.rs");
}
