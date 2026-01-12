// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use metrique::writer::BoxEntrySink;
use metrique::writer::test_util::{Inspector, TestEntrySink, test_entry_sink, to_test_entry};
use metrique::{
    CloseValue, LazySlot, OnParentDrop, RootEntry,
    timers::{
        EpochMicros, EpochMillis, EpochSeconds, Stopwatch, Timer, Timestamp, TimestampOnClose,
    },
    unit::{Millisecond, Second},
    unit_of_work::metrics,
};
use metrique_timesource::{
    ThreadLocalTimeSourceGuard, TimeSource, fakes::StaticTimeSource, set_time_source,
};

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: Timestamp,
    #[metrics(unit = Second)]
    time: Timer,

    #[metrics(unit = Millisecond)]
    explicit_time: Stopwatch,

    #[metrics(format = EpochMillis)]
    start_ts_millis: Timestamp,

    #[metrics(format = EpochMicros)]
    start_ts_micros: Timestamp,

    #[metrics(format = EpochSeconds)]
    start_ts_seconds: Timestamp,

    // use the default unit
    close_ts: TimestampOnClose,

    #[metrics(flatten)]
    subevent: LazySlot<Subevent>,

    #[metrics(format = EpochMillis)]
    optional_renamed: Option<Timestamp>,
}

#[derive(Default)]
#[metrics]
struct TimestampFormats {
    #[metrics(format = EpochSeconds)]
    seconds: Timestamp,

    #[metrics(format = EpochMillis)]
    millis: Timestamp,

    millis_default: Timestamp,

    #[metrics(format = EpochMicros)]
    micros: Timestamp,
}

#[metrics(prefix = "subevent_")]
#[derive(Default)]
struct Subevent {
    #[metrics(format = EpochMicros)]
    start_ts: Timestamp,

    #[metrics(format = EpochMicros)]
    end_ts: TimestampOnClose,
}

impl RequestMetrics {
    pub fn init(sink: BoxEntrySink) -> RequestMetricsGuard {
        RequestMetrics::default().append_on_drop(sink)
    }
}

fn start_timestamp() -> SystemTime {
    UNIX_EPOCH + Duration::from_micros(1_000_002)
}

#[tokio::test]
async fn metrics_flush_with_configurable_timestamp() {
    let (metric, inspector, _guard) = setup_with_tokio();
    drop(metric);
    assert_eq!(inspector.entries()[0].timestamp, Some(start_timestamp()));
}

fn setup_with_tokio() -> (RequestMetricsGuard, Inspector, ThreadLocalTimeSourceGuard) {
    tokio::time::pause();
    let _guard = set_time_source(TimeSource::tokio(start_timestamp()));
    let TestEntrySink { inspector, sink } = test_entry_sink();
    let metric = RequestMetrics::init(sink);
    (metric, inspector, _guard)
}

#[tokio::test]
async fn metric_timers_work() {
    let (mut metric, inspector, _guard) = setup_with_tokio();
    // the inner time should be 3 seconds

    let inner_timer = metric.explicit_time.start();
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(Duration::from_secs(3), inner_timer.stop());

    // go forward 1 more second. Total time should be 4, inner time should be 3
    tokio::time::advance(Duration::from_secs(1)).await;

    drop(metric);

    // for good measure, move time forward again, ensure we stopped the clock properly
    tokio::time::advance(Duration::from_secs(1)).await;

    let entries = inspector.entries();
    assert_eq!(entries[0].metrics["Time"], 4);
    assert_eq!(entries[0].metrics["ExplicitTime"], 3000);
}

#[tokio::test]
async fn timestamp_formats() {
    let (metric, inspector, _guard) = setup_with_tokio();
    tokio::time::advance(Duration::from_secs(3)).await;
    drop(metric);
    let entries = inspector.entries();
    assert_eq!(entries[0].values["StartTsMicros"], "1000002");
    assert_eq!(entries[0].values["StartTsMillis"], "1000.0020000000001");
    assert_eq!(entries[0].values["StartTsSeconds"], "1.000002");
}

#[tokio::test]
async fn explicit_timer_stop() {
    let (mut metric, inspector, _guard) = setup_with_tokio();
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(metric.time.stop(), Duration::from_secs(3));
    // advance time 3 more seconds before dropping
    tokio::time::advance(Duration::from_secs(3)).await;
    drop(metric);
    assert_eq!(inspector.entries()[0].metrics["Time"], 3.0);
}
#[tokio::test]
async fn subevents() {
    let (mut metrics, inspector, _guard) = setup_with_tokio();
    tokio::time::advance(Duration::from_secs(1)).await;
    let event = metrics
        .subevent
        .open(Subevent::default(), OnParentDrop::Discard)
        .unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    drop(event);
    // big delay between subevent ending and main ending
    tokio::time::advance(Duration::from_secs(5000)).await;
    drop(metrics);

    let entries = dbg!(inspector.entries());
    assert_eq!(
        entries[0].values["SubeventStartTs"],
        to_micros(start_timestamp() + Duration::from_secs(1))
    );
    assert_eq!(
        entries[0].values["SubeventEndTs"],
        to_micros(start_timestamp() + Duration::from_secs(6))
    );
}

#[test]
fn timestamp_format_test() {
    let ts = StaticTimeSource::at_time(UNIX_EPOCH + Duration::from_micros(1_001_001));
    let _guard = set_time_source(TimeSource::custom(ts));
    let entry = TimestampFormats::default().close();
    let entry = to_test_entry(RootEntry::new(entry));
    assert_eq!(entry.values["seconds"], "1.001001");
    assert_eq!(entry.values["millis"], "1001.001");
    assert_eq!(entry.values["micros"], "1001001");
}

fn to_micros(ts: SystemTime) -> String {
    ts.duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros()
        .to_string()
}
