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
    assert_eq!(entry.metrics["nested_count"].as_u64(), 2);
    assert_eq!(entry.metrics["nested_count"].unit, Unit::Count);
    assert_eq!(entry.metrics["nested"].as_u64(), 4);
    assert_eq!(entry.values["time_as_epoch_millis"], "1000.0");
    assert_eq!(entry.values["nested_foo"], "ZAB");
}
