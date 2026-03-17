// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::SystemTime;

use metrique::ServiceMetrics;
use metrique::json::Json;
use metrique::unit_of_work::metrics;
use metrique::writer::{
    AttachGlobalEntrySinkExt, Entry, EntryIoStreamExt, FormatExt, GlobalEntrySink,
};

#[derive(Debug)]
#[metrics]
struct RequestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: &'static str,
    status: &'static str,
    number_of_ducks: usize,
}

impl RequestMetrics {
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            timestamp: SystemTime::now(),
            operation,
            status: "INCOMPLETE",
            number_of_ducks: 0,
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[derive(Entry)]
#[entry]
struct Globals {
    region: String,
}

fn main() {
    let globals = Globals {
        // Generally, this is usually sourced from CLI args or the environment.
        region: "us-east-1".to_string(),
    };

    let _handle = ServiceMetrics::attach_to_stream(
        Json::new()
            .output_to_makewriter(|| std::io::stdout().lock())
            // All entries will contain `region` as a property.
            .merge_globals(globals),
    );

    let mut request = RequestMetrics::init("CountDucks");
    request.number_of_ducks += 10;
    request.status = "SUCCESS";

    /*
    {"timestamp":<millis>,"metrics":{"number_of_ducks":{"value":10}},"properties":{"region":"us-east-1","operation":"CountDucks","status":"SUCCESS"}}
    */
}
