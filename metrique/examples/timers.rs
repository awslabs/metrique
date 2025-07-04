// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, UNIX_EPOCH};

use metrique::timers::{EpochMicros, Stopwatch, Timer, Timestamp, TimestampOnClose};
use metrique::unit::Microsecond;
use metrique::unit_of_work::metrics;
use metrique::{LazySlot, OnParentDrop, SlotGuard};
use metrique_timesource::{TimeSource, set_time_source};
use metrique_writer::{AttachGlobalEntrySinkExt, EntryIoStreamExt, FormatExt};
use metrique_writer::{BoxEntrySink, Entry, GlobalEntrySink, sink::global_entry_sink};
use metrique_writer_format_emf::Emf;

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct RequestMetrics {
    operation: &'static str,

    // `timestamp` will use the format-specific destination for an entry timestamp
    #[metrics(timestamp)]
    timestamp: Timestamp,

    // timers are automatically started when created and stopped when the entry is dropped/closed
    time: Timer,

    // The stopwatch _must_ be explicitly started to report data
    #[metrics(unit = Microsecond)]
    load_data_time: Stopwatch,

    // for debugging or post-hoc analysis, sometimes it's helpful to have explicit timestamps.
    // These will be set as properties (not metrics)
    #[metrics(format = EpochMicros)]
    start_ts: Timestamp,

    // timestamp on close will record the time when the metric is closed.
    #[metrics(format = EpochMicros)]
    end_ts: TimestampOnClose,

    // if you have a subsegment w/ timings, `LazySlot` lets you initialize when you start processing the segment.
    #[metrics(flatten)]
    subevent: LazySlot<Subevent>,
}

// maybe you want to DRY timestamp formats across your program
type DefaultTimestampFormat = EpochMicros;

// By implementing the `ValueFormatter` trait, you can also define your own entirely custom formats.

#[metrics(prefix = "subevent_", subfield)]
#[derive(Default)]
struct Subevent {
    #[metrics(format = DefaultTimestampFormat)]
    start_ts: Timestamp,

    #[metrics(format = DefaultTimestampFormat)]
    end_ts: TimestampOnClose,
}

impl RequestMetrics {
    fn init(operation: &'static str, sink: BoxEntrySink) -> RequestMetricsGuard {
        RequestMetrics {
            operation,
            ..Default::default()
        }
        .append_on_drop(sink)
    }
}

#[derive(Entry)]
#[entry(rename_all = "PascalCase")]
struct Globals {
    region: String,
}

#[tokio::main]
async fn main() {
    // for clarity, mock SystemTime to be `0` so that the timestamps are readable
    let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
    tracing_subscriber::fmt::init();
    let _handler = ServiceMetrics::attach_to_stream(
        Emf::all_validations("MyApp".into(), vec![vec![]])
            .output_to(std::io::stdout())
            .merge_globals(Globals {
                region: "us-east-1".into(),
            }),
    );
    handle_request(ServiceMetrics::sink()).await

    // EXAMPLE OUTPUT:
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"MyApp","Dimensions":[[]],"Metrics":[{"Name":"Time","Unit":"Milliseconds"},{"Name":"LoadDataTime","Unit":"Microseconds"}]}],"Timestamp":1},"Time":181.30229,"LoadDataTime":46443.717,"Region":"us-east-1","Operation":"GoFishing","StartTs":"1898","EndTs":"183202","SubeventStartTs":"59617","SubeventEndTs":"183192"}
    */
}

async fn handle_request(sink: BoxEntrySink) {
    let mut metrics = RequestMetrics::init("GoFishing", sink);
    // first lets do some initial processing
    tokio::time::sleep(Duration::from_millis(10)).await;

    {
        let _load_data = metrics.load_data_time.start();
        load_data().await;
        // when the scope exits, `_load_data` is dropped and stops the timer.
        // You can also explicitly call `stop`
    }

    process_subevent(
        metrics
            .subevent
            .open(Subevent::default(), OnParentDrop::Discard)
            .unwrap(),
    )
    .await;
}

async fn process_subevent(_submetrics: SlotGuard<Subevent>) {
    tokio::time::sleep(Duration::from_millis(123)).await;
}

async fn load_data() {
    // ...
    tokio::time::sleep(Duration::from_millis(45)).await;
}
