use metrique::{
    CloseValue, RootEntry,
    test_util::{TestEntrySink, test_entry_sink},
    unit_of_work::metrics,
};
use metrique_writer::{Entry, EntrySink};

#[metrics(value(string))]
enum Operation {
    CountDucks,
    CountGeese,
}

#[metrics(subfield)]
struct GeneralMetrics {
    #[metrics(sample_group)]
    operation: Operation,
    #[metrics(sample_group, name = "APIStatus")]
    api_status: Status,
}

#[metrics(value, sample_group)]
struct Status {
    status: &'static str,
}

#[metrics(rename_all = "PascalCase")]
struct MyMetric {
    #[metrics(flatten)]
    general: GeneralMetrics,
    bird_species: &'static str,
    number_of_birds: usize,
}

#[derive(Entry)]
pub struct StatusEntry {
    #[entry(sample_group)]
    status: &'static str,
}

#[metrics(rename_all = "PascalCase")]
struct FlattenEntry {
    #[metrics(sample_group)]
    operation: Operation,
    #[metrics(flatten_entry, no_close)]
    status: StatusEntry,
}

#[test]
fn test_sample_group_my_metric() {
    let metric = MyMetric {
        general: GeneralMetrics {
            operation: Operation::CountDucks,
            api_status: Status { status: "SUCCESS" },
        },
        bird_species: "Mallard",
        number_of_birds: 0,
    };
    let entry = RootEntry::new(metric.close());
    let sample_group = entry
        .sample_group()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<Vec<_>>();
    assert_eq!(
        sample_group,
        vec![
            ("Operation".to_string(), "CountDucks".to_string()),
            ("APIStatus".to_string(), "SUCCESS".to_string())
        ]
    );
    let TestEntrySink { inspector, sink } = test_entry_sink();
    sink.append(entry);
    assert_eq!(inspector.get(0).values["Operation"], "CountDucks");
    assert_eq!(inspector.get(0).values["APIStatus"], "SUCCESS");
}

#[test]
fn test_sample_group_flatten_entry() {
    let metric = FlattenEntry {
        operation: Operation::CountGeese,
        status: StatusEntry { status: "FAILURE" },
    };
    let entry = RootEntry::new(metric.close());
    let sample_group = entry
        .sample_group()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<Vec<_>>();
    // status is not inflected since it is in a flatten entry
    assert_eq!(
        sample_group,
        vec![
            ("Operation".to_string(), "CountGeese".to_string()),
            ("status".to_string(), "FAILURE".to_string())
        ]
    );
    let TestEntrySink { inspector, sink } = test_entry_sink();
    sink.append(entry);
    assert_eq!(inspector.get(0).values["Operation"], "CountGeese");
    assert_eq!(inspector.get(0).values["status"], "FAILURE");
}
