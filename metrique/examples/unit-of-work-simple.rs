// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This is a simple example showing basics of unit of work metrics.
//!
//! See `./unit-of-work-owned-child-metrics.rs` if you want to pass around owned inner structs
//! rather than references.
//!
//! See `./unit-of-work-fanout.rs` if you want to write to a single metric entry from multiple
//! child tasks in the background.

use core::{default::Default, time::Duration};
use std::time::SystemTime;

use metrique::{
    unit::{Count, Microsecond},
    unit_of_work::metrics,
};
use metrique_writer::{
    AttachGlobalEntrySinkExt, Entry, EntryIoStreamExt, FormatExt, GlobalEntrySink,
    sink::global_entry_sink,
};
use metrique_writer_format_emf::Emf;
global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    // A metric timestamp that we set explicitly to capture the start of the request.
    // If you don't set it, instead it will be implicitly calculated when the metric
    // eventually is flushed, which will be after request processing, any waiting
    // for child tasks to close, any time in a background queue, etc.
    #[metrics(timestamp)]
    timestamp: SystemTime,

    // you can nest structs for readability and flatten them to a single entry
    #[metrics(flatten)]
    keys: Keys,

    // a simple count field (you can add units. Units are defined in `amzn_metrics::unit`).
    // It is also possible to define your own.
    #[metrics(unit = Count)]
    number_of_ducks: usize,

    #[metrics(unit = Microsecond)]
    response_time: Duration,
}

impl RequestMetrics {
    /// The fundamental api for `#[metrics]` is append on drop: You create a new metric struct that is already tied to the relevant sink.
    /// When all handles have been dropped, it is flushed to the sink. This normally would happen at the end of the request, but could happen later.
    ///
    /// This function generates a new instance of the metric, which flushes to ServiceMetrics
    /// when it goes out of scope
    fn init(keys: Keys) -> RequestMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            keys,
            number_of_ducks: 0,
            response_time: Duration::default(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[metrics]
struct Keys {
    request_id: String,
    cluster_id: String,
    operation: &'static str,
}

#[derive(Entry)]
#[entry(rename_all = "PascalCase")]
struct Globals {
    region: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let globals = Globals {
        region: "us-east-1".into(),
    };
    // to write to a rolling appender instead
    // let writer = tracing_appender::rolling::RollingFileAppender::new(
    //     tracing_appender::rolling::Rotation::MINUTELY,
    //     &service_log_dir,
    //     "service_log.log",
    // );
    let writer = std::io::stdout;

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]])
            .output_to_makewriter(writer)
            .merge_globals(globals),
    );
    handle_request("a").await;
    handle_request("b").await;

    // TODO: add example EMF output

    // EXAMPLE OUTPUT
    /*
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"NumberOfDucks","Unit":"Count"},{"Name":"ResponseTime","Unit":"Microseconds"}]}],"Timestamp":1750100200121},"NumberOfDucks":2,"ResponseTime":500000,"Region":"us-east-1","RequestId":"a","ClusterId":"cluster1234","Operation":"GET"}
    {"_aws":{"CloudWatchMetrics":[{"Namespace":"Ns","Dimensions":[[]],"Metrics":[{"Name":"NumberOfDucks","Unit":"Count"},{"Name":"ResponseTime","Unit":"Microseconds"}]}],"Timestamp":1750100200121},"NumberOfDucks":2,"ResponseTime":500000,"Region":"us-east-1","RequestId":"b","ClusterId":"cluster1234","Operation":"GET"}    ---
         */
}

async fn handle_request(request_id: &str) {
    let mut metrics = RequestMetrics::init(Keys {
        cluster_id: "cluster1234".to_string(),
        request_id: request_id.to_owned(),
        operation: "GET",
    });

    // Regular fields can be mutated in place. No performance penalty, no atomics.
    // The downside is that mutating these fields requires `&mut self`.
    metrics.number_of_ducks += 1;

    // If lifetimes/references are simple, you can just pass the entry to child functions by reference
    do_some_work(&mut metrics).await;

    // you can add dimensions to individual fields, which will pull them out
    // into their own entry for every key/value set of dimensions
    metrics.response_time = Duration::from_millis(500);
}

async fn do_some_work(metrics: &mut RequestMetrics) {
    metrics.number_of_ducks += 1;
}
