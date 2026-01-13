//! Example: Split Aggregation Pattern with Sampling
//!
//! This example demonstrates using `SplitSink` to send the same data to multiple
//! destinations. We aggregate metrics for precise counts while also emitting sampled
//! raw events for debugging and tracing.

use metrique::ServiceMetrics;
use metrique::emf::Emf;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::Histogram;
use metrique_aggregation::sink::{EntrySinkAsAggregateSink, SplitSink};
use metrique_aggregation::traits::{AggregateStrategy, Key};
use metrique_aggregation::value::Sum;
use metrique_aggregation::{aggregate, aggregator::KeyedAggregator, sink::WorkerSink};
use metrique_writer::sample::SampledFormatExt;
use metrique_writer::sink::FlushImmediatelyBuilder;
use metrique_writer::value::ToString;
use metrique_writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink};
use std::borrow::Cow;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;

#[aggregate(ref)]
#[metrics(emf::dimension_sets = [["has_errors", "endpoint"], ["endpoint"]])]
struct ApiCall {
    #[aggregate(key)]
    endpoint: String,

    #[aggregate(strategy = Sum)]
    request_count: u64,

    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = Sum)]
    errors: u64,
}

struct AggregateByErrorsEndpoint;

impl AggregateStrategy for AggregateByErrorsEndpoint {
    type Source = ApiCallEntry;

    type Key = AggregateByErrorsEndpoint;
}

#[derive(Debug, Hash, Clone, PartialEq, Eq)]
#[metrics]
pub struct ByErrorsEndpoint<'a> {
    #[metrics(format = ToString)]
    has_errors: bool,
    endpoint: Cow<'a, str>,
}

impl Key<ApiCallEntry> for AggregateByErrorsEndpoint {
    type Key<'a> = ByErrorsEndpoint<'a>;

    fn from_source(source: &ApiCallEntry) -> Self::Key<'_> {
        #[expect(deprecated)]
        ByErrorsEndpoint {
            has_errors: source.errors > 0,
            endpoint: Cow::Borrowed(&source.endpoint),
        }
    }

    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
        ByErrorsEndpoint {
            has_errors: key.has_errors,
            endpoint: Cow::Owned(key.endpoint.clone().into_owned()),
        }
    }

    fn static_key_matches<'a>(owned: &Self::Key<'static>, borrowed: &Self::Key<'a>) -> bool {
        owned == borrowed
    }
}

// Simulated API call
async fn make_api_call(endpoint: &str) -> Result<(), String> {
    // Simulate varying latencies
    let delay = match endpoint {
        "GetUser" => 15,
        "UpdateUser" => 45,
        "DeleteUser" => 30,
        "ListUsers" => 100,
        _ => 25,
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;

    // Simulate occasional errors
    if endpoint == "DeleteUser" && rand::random::<f32>() < 0.2 {
        Err("Permission denied".to_string())
    } else {
        Ok(())
    }
}

async fn api_service(mut requests: mpsc::Receiver<String>) {
    // Create aggregator for precise metrics
    let aggregate_by_endpoint = KeyedAggregator::<ApiCall>::new(ServiceMetrics::sink());
    // also aggregate by errors
    let aggregate_by_endoint_errors =
        KeyedAggregator::<AggregateByErrorsEndpoint>::new(ServiceMetrics::sink());

    // Create a second sink with sampling for raw events
    // This demonstrates sending sampled raw events to a separate destination
    let raw_stream = Emf::builder("RawRequestMetrics".to_string(), vec![vec![]])
        .skip_all_validations(true)
        .build()
        .with_sampling()
        .sample_by_fixed_fraction(0.5) // Sample 50% of raw events
        .output_to_makewriter(|| std::io::stdout().lock());

    let raw_sink = FlushImmediatelyBuilder::new().build_boxed(raw_stream);

    // Create raw sink for individual events
    let raw = EntrySinkAsAggregateSink::new(raw_sink);

    // Combine them with SplitSink
    let split = SplitSink::new(
        aggregate_by_endpoint,
        SplitSink::new(aggregate_by_endoint_errors, raw),
    );
    let sink = WorkerSink::new(split, Duration::from_millis(500));

    info!("API service started. Processing requests...\n");

    while let Some(endpoint) = requests.recv().await {
        let start = std::time::Instant::now();
        let result = make_api_call(&endpoint).await;
        let latency = start.elapsed();

        // Send to both aggregated and raw sinks
        ApiCall {
            endpoint: endpoint.clone(),
            request_count: 1,
            latency,
            errors: if result.is_err() { 1 } else { 0 },
        }
        .close_and_merge(sink.clone());
    }

    // Flush both sinks
    info!("\nFlushing metrics...");
    sink.flush().await;
}

#[tokio::main]
async fn main() {
    // Initialize tracing to see validation errors
    tracing_subscriber::fmt::init();

    // Attach global EMF sink
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::builder("RequestMetrics".to_string(), vec![vec![]])
            .skip_all_validations(true)
            .build()
            .output_to_makewriter(|| std::io::stdout().lock()),
    );
    let (tx, rx) = mpsc::channel(100);

    // Spawn the API service
    let service = tokio::spawn(api_service(rx));

    // Simulate incoming API requests
    let requests = vec![
        "GetUser",
        "GetUser",
        "GetUser",
        "UpdateUser",
        "UpdateUser",
        "DeleteUser",
        "DeleteUser",
        "DeleteUser",
        "ListUsers",
        "GetUser",
        "UpdateUser",
        "DeleteUser",
    ];

    for endpoint in requests {
        tx.send(endpoint.to_string()).await.unwrap();
    }

    // Close the channel to signal completion
    drop(tx);

    // Wait for service to finish
    service.await.unwrap();
}
