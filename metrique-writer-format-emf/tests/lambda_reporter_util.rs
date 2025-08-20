use std::time::UNIX_EPOCH;

use metrics_024::counter;
use metrique_timesource::{TimeSource, fakes::StaticTimeSource};

pub async fn perform_test(get: impl FnOnce() -> String) {
    let _guard = metrique_timesource::set_time_source(TimeSource::custom(
        StaticTimeSource::at_time(UNIX_EPOCH + std::time::Duration::from_secs(86_400)),
    ));
    counter!("my_counter").increment(3);
    counter!("my_counter", "label" => "value1").increment(1);
    counter!("my_counter", "label" => "value2").increment(2);
    metrique_metricsrs::lambda_reporter::flush_metrics()
        .await
        .unwrap();

    // the metrics emitter iterates over a hashmap so the order is unpredictable
    let dump = get();
    let mut contents: Vec<_> = dump.split('\n').collect();
    contents.sort();
    let contents = contents.join("\n");

    let expected = r#"
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[["label"]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":86400000},"label":"value1","my_counter":1}
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[["label"]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":86400000},"label":"value2","my_counter":2}
{"_aws":{"CloudWatchMetrics":[{"Namespace":"MyNS","Dimensions":[[]],"Metrics":[{"Name":"my_counter"}]}],"Timestamp":86400000},"my_counter":3}"#;
    assert_eq!(expected, contents);
}
