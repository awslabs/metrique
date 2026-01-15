//! Example: Embedded Aggregation Pattern
//!
//! This example demonstrates using `Aggregate<T>` to aggregate multiple sub-operations
//! within a single unit of work. A distributed query fans out to multiple backend shards,
//! and we aggregate all the backend call metrics into a single entry.

use metrique::DefaultSink;
use metrique::emf::Emf;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique::writer::BoxEntrySink;
use metrique::writer::{FormatExt, sink::FlushImmediatelyBuilder};
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::Aggregate;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::value::{KeepLast, Sum};
use metrique_writer::{AttachGlobalEntrySinkExt, GlobalEntrySink};
use metrique_writer_core::global_entry_sink;
use std::sync::Arc;
use std::time::Duration;

#[aggregate(ref)]
#[metrics]
struct BackendCall {
    #[aggregate(strategy = Sum)]
    requests_made: usize,

    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = Sum)]
    errors: u64,

    // This field needs to be marked `clone` in order to use `aggregate(ref)`.
    // This example require aggregate ref because we are using BackendCall both in aggregation
    // and emitting the same record as a non-aggregated event
    #[aggregate(strategy = KeepLast, clone)]
    error_message: Option<String>,

    // this field is ignored for aggregation, but preserved when using BackendCall in unit-of-work
    // metrics. This means that when the raw events are emitted (in this example on slow reequests and errors)
    // they will contain the query_id so it can be traced back to the main record
    #[aggregate(ignore)]
    query_id: Arc<String>,
}

#[metrics(rename_all = "PascalCase", emf::dimension_sets = [["QueryId"]])]
struct DistributedQuery {
    #[metrics(no_close)]
    query_id: Arc<String>,
    #[metrics(flatten)]
    backend_calls: Aggregate<BackendCall>,
}

// Simulated backend call
async fn call_backend(shard: &str, _query: &str) -> Result<String, String> {
    // Simulate varying latencies
    let delay = match shard {
        "shard1" => 45,
        "shard2" => 67,
        "shard3" => 52,
        "shard4" => 71,
        "shard5" => 58,
        _ => 50,
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;

    // Simulate occasional errors
    if shard == "shard3" {
        Err("Connection timeout".to_string())
    } else {
        Ok(format!("Results from {}", shard))
    }
}

async fn execute_distributed_query(query: &str, sink: BoxEntrySink) {
    let mut metrics = DistributedQuery {
        query_id: Arc::new(uuid::Uuid::new_v4().to_string()),
        backend_calls: Aggregate::default(),
    }
    .append_on_drop(sink);
    let query_id = metrics.query_id.clone();

    // Fan out to 100 backend shards
    let sampled_calls = SampledApiCalls::sink();
    for shard in 0..100 {
        let start = std::time::Instant::now();
        let result = call_backend(&format!("shard{shard}"), query).await;
        let latency = start.elapsed();

        // Insert each backend call into the aggregator
        // If it was a slow call or was an error, log the non-aggregated event as well
        let should_sample = latency > Duration::from_millis(70) || result.is_err();
        let backend_call = BackendCall {
            requests_made: 1,
            latency,
            errors: if result.is_err() { 1 } else { 0 },
            error_message: result.err().map(|err| format!("{err}")),
            query_id: Arc::clone(&query_id),
        };
        if should_sample {
            metrics
                .backend_calls
                .insert_and_send_to(backend_call, &sampled_calls);
        } else {
            metrics.backend_calls.insert(backend_call);
        }
    }

    // Metrics automatically emitted when dropped
}

global_entry_sink! { SampledApiCalls }

#[tokio::main]
async fn main() {
    // Create EMF sink that outputs to stdout
    let emf_stream = Emf::builder("DistributedQueryMetrics".to_string(), vec![vec![]])
        .build()
        .output_to_makewriter(|| std::io::stdout().lock());

    let emf: DefaultSink = FlushImmediatelyBuilder::new().build_boxed(emf_stream);

    // You can sample on the stream, but in this case, we do sampling during emission
    let sampled_stream = Emf::builder("SampledBackendCalls".to_string(), vec![vec![]])
        .skip_all_validations(true)
        .build()
        .output_to_makewriter(|| std::io::stdout().lock());
    let _handle = SampledApiCalls::attach_to_stream(sampled_stream);

    for _i in 0..5 {
        execute_distributed_query("SELECT * FROM users WHERE active = true", emf.clone()).await;
    }

    // This outputs both sampled backend calls like:
    // {"_aws":{"CloudWatchMetrics":[{"Namespace":"SampledBackendCalls","Dimensions":[[]],"Metrics":[{"Name":"requests_made"},{"Name":"latency","Unit":"Milliseconds"},{"Name":"errors"}]}],"Timestamp":1768409318340},"requests_made":1,"latency":72.585458,"errors":0,"query_id":"566bcf5c-a912-41b4-aa88-054751981433"}
    //
    // As well as aggregated stats:
    // {"_aws":{"CloudWatchMetrics":[{"Namespace":"DistributedQueryMetrics","Dimensions":[["QueryId"]],"Metrics":[{"Name":"RequestsMade"},{"Name":"Latency","Unit":"Milliseconds"},{"Name":"Errors"}]}],"Timestamp":1768409312778},"RequestsMade":100,"Latency":{"Values":[46.9990234375,50.9990234375,52.9990234375,54.9990234375,56.9990234375,60.9990234375,69.9990234375,73.9990234375],"Counts":[1,37,52,5,1,2,1,1]},"Errors":1,"QueryId":"cbd0e555-20a3-40e5-b4bd-f02043f8ecc9"}
}
