#![deny(missing_docs)]

//! Crate docs

// test that we don't break missing_docs

use metrique::unit_of_work::metrics;

#[metrics]
/// Foo
pub struct MyMetric {
    #[metrics(sample_group)]
    operation: Operation,
}

#[metrics(value, sample_group)]
/// Bar
pub struct Operation(OperationInner);

#[metrics(value(string))]
/// Baz
pub enum OperationInner {
    /// Foo
    Foo,
    /// Bar
    Bar,
}

fn main() {}
