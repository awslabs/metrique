use metrique::unit_of_work::metrics;
use metrique::{CloseValue, RootEntry};
use metrique_writer::test_util;

#[metrics(value(string), rename_all = "snake_case")]
enum Foo {
    Foo,
    Bar,
    #[metrics(name = "ZAB")]
    Baz,
    BarBaz,
}

#[metrics]
struct Metrics {
    f1: Foo,
    f2: Foo,
    f3: Foo,
    f4: Foo,
}

#[test]
fn string_value() {
    let metrics = Metrics {
        f1: Foo::Foo,
        f2: Foo::Bar,
        f3: Foo::Baz,
        f4: Foo::BarBaz,
    };
    let entry = test_util::to_test_entry(RootEntry::new(metrics.close()));
    assert_eq!(entry.values["f1"], "foo");
    assert_eq!(entry.values["f2"], "bar");
    assert_eq!(entry.values["f3"], "ZAB");
    assert_eq!(entry.values["f4"], "bar_baz");
}
