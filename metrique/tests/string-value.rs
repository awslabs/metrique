use metrique_timesource::{TimeSource, set_time_source};
use std::time::{Duration, UNIX_EPOCH};

use metrique::timers::{EpochMillis, Timestamp};
use metrique::unit::Count;
use metrique::unit_of_work::metrics;
use metrique::writer::{Unit, test_util};
use metrique::{CloseValue, RootEntry};

#[metrics(value(string), rename_all = "snake_case")]
enum Foo {
    Foo,
    Bar,
    #[metrics(name = "ZAB")]
    Baz,
    BarBaz,
}

#[metrics(value)]
struct NestedValue(u32);

#[metrics(value)]
struct NestedCount(#[metrics(unit=Count)] u32);

#[metrics(value)]
struct NestedFoo(Foo);

#[metrics(value)]
struct TimeAsEpochMillis {
    #[metrics(format=EpochMillis)]
    time: Timestamp,
}

#[metrics(value, sample_group)]
struct Operation(&'static str);

#[metrics(value)]
struct Empty {}

#[metrics]
struct Metrics {
    f1: Foo,
    f2: Foo,
    f3: Foo,
    f4: Foo,
    nested: NestedValue,
    nested_foo: NestedFoo,
    time_as_epoch_millis: TimeAsEpochMillis,
    nested_count: NestedCount,
    empty: Empty,
}

#[tokio::test(start_paused = true)]
async fn string_value() {
    let _guard = set_time_source(TimeSource::tokio(UNIX_EPOCH + Duration::from_secs(1)));
    let metrics = Metrics {
        f1: Foo::Foo,
        f2: Foo::Bar,
        f3: Foo::Baz,
        f4: Foo::BarBaz,
        nested: NestedValue(4),
        nested_foo: NestedFoo(Foo::Baz),
        nested_count: NestedCount(2),
        time_as_epoch_millis: TimeAsEpochMillis {
            time: Timestamp::now(),
        },
        empty: Empty {},
    };
    let entry = test_util::to_test_entry(RootEntry::new(metrics.close()));
    assert_eq!(entry.values["f1"], "foo");
    assert_eq!(entry.values["f2"], "bar");
    assert_eq!(entry.values["f3"], "ZAB");
    assert_eq!(entry.values["f4"], "bar_baz");
    assert_eq!(entry.metrics["nested_count"], 2);
    assert_eq!(entry.metrics["nested_count"].unit, Unit::Count);
    assert_eq!(entry.metrics["nested"], 4);
    assert_eq!(entry.values["time_as_epoch_millis"], "1000.0");
    assert_eq!(entry.values["nested_foo"], "ZAB");
}

#[test]
fn value_string_entry_auto_derives_debug_clone_copy() {
    #[metrics(value(string))]
    enum AutoDerived {
        Alpha,
    }

    // The closed value should also have Debug, Clone, Copy auto-derived
    let closed = metrique::CloseValue::close(AutoDerived::Alpha);

    let debug_str = format!("{:?}", closed);
    assert_eq!(debug_str, "Alpha");

    let cloned = closed.clone();
    assert_eq!(format!("{:?}", cloned), "Alpha");

    let copied = closed;
    let still_usable_after_copy = closed;
    assert_eq!(format!("{:?}", copied), "Alpha");
    assert_eq!(format!("{:?}", still_usable_after_copy), "Alpha");
}

#[test]
fn value_string_extra_user_derives_before_metrics() {
    // User derives whatever they need on the base enum themselves.
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    #[metrics(value(string))]
    enum Priority {
        #[default]
        Low,
    }

    let p = Priority::Low;
    let _ = format!("{:?}", p);
    let copied = p;
    assert_eq!(p, copied);
    assert_eq!(Priority::default(), Priority::Low);

    let closed = metrique::CloseValue::close(p);
    assert_eq!(format!("{:?}", closed), "Low");
}

#[test]
fn value_string_user_derives_after_metrics_preserved() {
    // All user derives (including Debug/Clone/Copy) are preserved as-is.
    #[metrics(value(string))]
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    enum Priority {
        #[default]
        Low,
    }

    let p = Priority::Low;
    let _ = format!("{:?}", p);
    let copied = p;
    assert_eq!(p, copied);
    assert_eq!(Priority::default(), Priority::Low);

    let closed = metrique::CloseValue::close(p);
    assert_eq!(format!("{:?}", closed), "Low");
}
