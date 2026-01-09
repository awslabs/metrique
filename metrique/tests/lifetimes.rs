use assert2::check;
use metrique::{
    test_util::{TestEntrySink, test_entry_sink, test_metric},
    unit_of_work::metrics,
};
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

#[metrics(value)]
struct MetricsValueLifetime<'a> {
    a: &'a str,
}

#[metrics]
struct UtilizeValue {
    v: MetricsValueLifetime<'static>,
}

/* Currently impossible -- we need a setting to not attempt to generate the `Ref` impl I think?
#[metrics(value)]
struct MetricsValueLifetimeCow<'a> {
    cow: Cow<'a, str>,
}
*/

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

#[test]
fn static_metrics_append_on_drop() {
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let foo: Foo<'static> = Foo {
        a: Cow::Borrowed(&"123"),
        b: 5,
        c: Cow::Owned("1234".to_string()),
    };
    let mut guard = foo.append_on_drop(sink);
    guard.b = 10;
    drop(guard);
    let entry = inspector.entries()[0].clone();
    check!(entry.metrics["b"] == 10);
    check!(entry.values["a"] == "123");
    check!(entry.values["c"] == "1234");
}

#[test]
fn value_test() {
    let metric = UtilizeValue {
        v: MetricsValueLifetime { a: "123" },
    };
    check!(test_metric(metric).values["v"] == "123");
}
