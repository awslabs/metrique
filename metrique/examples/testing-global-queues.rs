// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{sync::Arc, time::Duration};

use metrique::emf::Emf;
use metrique::writer::{
    AttachGlobalEntrySinkExt, Entry, EntryIoStreamExt, FormatExt, GlobalEntrySink,
    sink::global_entry_sink,
};
use metrique::{
    timers::{Stopwatch, Timer, Timestamp},
    unit::Millisecond,
    unit_of_work::metrics,
};

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct RequestMetrics {
    operation: &'static str,

    #[metrics(timestamp)]
    timestamp: Timestamp,

    // time field, records total time of the operation
    time: Timer,
    number_of_ducks: usize,

    #[metrics(unit = Millisecond)]
    duck_counter_time: Stopwatch,
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        Self {
            operation,
            ..Default::default()
        }
        .append_on_drop(ServiceMetrics::sink()) // Uses global sink directly
    }
}

struct ServerState {
    // No need to store sink - uses global sink
}

#[derive(Entry)]
struct Globals {
    region: String,
    cell: String,
}

async fn look_at_sky(_state: Arc<ServerState>) {
    let mut metrics = RequestMetrics::init("LookAtSky");
    let number_of_ducks = {
        let _guard = metrics.duck_counter_time.start();
        count_ducks().await
    };
    metrics.number_of_ducks += number_of_ducks;
    // sleeping to produce some time for metrics. obviously don't do this in real code.
    tokio::time::sleep(Duration::from_millis(234)).await;
}

async fn count_ducks() -> usize {
    // sleeping to produce some time for metrics. obviously don't do this in real code.
    tokio::time::sleep(Duration::from_millis(1234)).await;
    42
}

#[tokio::main]
async fn main() {
    let globals = Globals {
        region: "us-east-1".to_string(),
        cell: "5".to_string(),
    };
    // in prod, initialize it to use the global sink
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("MyApp".to_string(), vec![vec![]])
            .output_to_makewriter(|| std::io::stdout().lock())
            .merge_globals(globals),
    );

    let app = Arc::new(ServerState {});
    // start your app, handle requests, etc.
    look_at_sky(app).await;
}

#[cfg(test)]
mod test {
    use std::time::UNIX_EPOCH;

    use metrique::test_util::{self, TestEntrySink};
    use metrique_timesource::{TimeSource, set_time_source};

    // If you want the times produced by tokio to exactly line up, you need start_paused=true
    #[tokio::test(start_paused = true)]
    async fn looking_at_sky_produces_metrics() {
        let _mock_time = set_time_source(TimeSource::tokio(UNIX_EPOCH));

        // Create test sink and install thread-local override
        let TestEntrySink { inspector, sink } = test_util::test_entry_sink();
        let _guard = crate::ServiceMetrics::set_test_sink(sink);

        // Code under test uses global sink, but metrics go to test sink
        let app = std::sync::Arc::new(crate::ServerState {});
        super::look_at_sky(app.clone()).await;

        let entry = inspector.get(0);
        assert_eq!(entry.values["Operation"], "LookAtSky");
        assert_eq!(entry.metrics["NumberOfDucks"], 42);
        assert_eq!(entry.timestamp, Some(UNIX_EPOCH));
        assert_eq!(entry.metrics["DuckCounterTime"], 1234);
        assert_eq!(entry.metrics["Time"], 1468);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn multithreaded_runtime_sink() {
        let _mock_time = set_time_source(TimeSource::tokio(UNIX_EPOCH));

        // Use runtime-scoped guard for multi-threaded tests
        let TestEntrySink { inspector, sink } = test_util::test_entry_sink();
        let _guard = crate::ServiceMetrics::set_test_sink_on_runtime(sink);

        let app = std::sync::Arc::new(crate::ServerState {});

        // Spawn multiple tasks across threads
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let app = app.clone();
                tokio::spawn(async move {
                    super::look_at_sky(app).await;
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        let entries = inspector.entries();
        assert_eq!(entries.len(), 3);
        for entry in entries {
            assert_eq!(entry.values["Operation"], "LookAtSky");
            assert_eq!(entry.metrics["NumberOfDucks"], 42);
        }
    }
}
