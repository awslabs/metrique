// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics(rename_all = "snake_case", bad_root_attr, bad_root_attr_eq = "foo")]
struct MetricsWithInvalidUnit {
    operation: &'static str,

    #[metrics(name = "a", name = "b")]
    duplicate_name: usize,

    // This should fail because NonExistentUnit is not a valid unit
    #[metrics(unit = Seconds, unit = Minutes)]
    size: usize,

    #[metrics(flatten, name = "foo")]
    subfield: SubMetric,

    #[metrics(name = 5)]
    bad_name: usize,

    #[metrics(nme = "foo")]
    not_valid: usize,

    #[metrics(name = "")]
    bad_name_2: usize,

    #[metrics(name = "a b")]
    bad_name_3: usize,

    #[metrics(ignore, no_close)]
    ignore_no_close: usize,

    #[metrics(prefix = "foo")]
    prefix_no_flatten: usize,

    #[metrics(flatten, prefix = "foo:")]
    prefix_bad_character: usize,

    #[metrics(flatten, prefix = "foo", exact_prefix = "bar")]
    prefix_and_exact: SubMetric,
}

#[metrics(rename_all = "snake_case")]
struct SubMetric {
    a: usize,
}

#[metrics(prefix = "foo", exact_prefix = "foo")]
struct PrefixAndExact {
    a: usize,
}

#[metrics(prefix = "foo@")]
struct PrefixBadCharacter {
    a: usize,
}

#[metrics(sample_group)]
struct SampleGroupTopLevelEntry {
    foo: usize,
}

#[metrics]
struct SampleGroupIgnore {
    #[metrics(ignore, sample_group)]
    foo: &'static str,
    #[metrics(sample_group, ignore)]
    foo2: &'static str,
}

#[metrics(value)]
struct SampleGroupFieldOnStruct {
    #[metrics(sample_group)]
    field: &'static str,
}

#[metrics(value, sample_group)]
struct SampleGroupValueAllIgnore {
    #[metrics(ignore)]
    ignore: u32,
}

#[metrics(prefix="foo")]
struct MetricPrefixNoDelim {
    field: &'static str,
}

#[metrics(prefix="foo-bar")]
struct MetricPrefixNoDelimWithSnake {
    field: &'static str,
}

fn main() {}
