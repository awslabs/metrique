use assert2::check;
use metrique::{test_util::test_metric, unit_of_work::metrics};
use std::borrow::Cow;

#[metrics]
pub struct Foo<'a> {
    a: Cow<'a, str>,
    b: usize,
    c: Cow<'a, String>,
}

#[metrics]
enum FooEnum<'a> {
    Variant1(#[metrics(flatten)] Foo<'a>),
    Variant2 { v: &'a str },
}

#[metrics]
struct MultipleLifetimes<'a, 'b> {
    a: Cow<'a, str>,
    b: Cow<'b, str>,
}

#[test]
fn metrics_work() {
    let foo = Foo {
        a: Cow::Borrowed(&"123"),
        b: 5,
        c: Cow::Owned("1234".to_string()),
    };

    let entry = test_metric(foo);
    check!(entry.values["a"] == "123");
}
