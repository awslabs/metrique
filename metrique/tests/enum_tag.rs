use metrique::unit_of_work::metrics;
use metrique::writer::{Entry, test_util::test_metric};
use metrique::{CloseValue, RootEntry};

// Tag with prefix and rename_all - using name_exact to avoid inflection
#[metrics(tag(name_exact = "op"), prefix = "api_", rename_all = "snake_case")]
enum WithPrefixExact {
    ReadData { count: u32 },
}

#[test]
fn tag_field_exact_ignores_inflection() {
    let entry = test_metric(WithPrefixExact::ReadData { count: 42 });

    // Tag field name_exact ignores prefix and rename_all
    // Tag value respects rename_all but not prefix
    assert_eq!(entry.values["op"], "read_data");
    // count field gets both prefix and rename_all
    assert_eq!(entry.metrics["api_count"].as_u64(), 42);
}

// Tag with prefix and rename_all - using name to apply inflection
#[metrics(tag(name = "op"), prefix = "api_", rename_all = "snake_case")]
enum WithPrefixInflectable {
    ReadData { count: u32 },
}

#[test]
fn tag_field_name_respects_inflection() {
    let entry = test_metric(WithPrefixInflectable::ReadData { count: 42 });

    // Tag field name respects prefix and rename_all
    // Tag value respects rename_all but not prefix
    assert_eq!(entry.values["api_op"], "read_data");
    // count field gets both prefix and rename_all
    assert_eq!(entry.metrics["api_count"].as_u64(), 42);
}

// Tag with variant name override
#[metrics(tag(name = "op"))]
enum CustomName {
    #[metrics(name = "custom_read")]
    Read { bytes: usize },
    #[metrics(name = "custom_write")]
    Write(#[metrics(ignore)] usize),
}

#[test]
fn tag_respects_variant_name() {
    let entry = test_metric(CustomName::Read { bytes: 512 });
    assert_eq!(entry.values["op"], "custom_read");
    assert_eq!(entry.metrics["bytes"].as_u64(), 512);

    let entry = test_metric(CustomName::Write(1024));
    assert_eq!(entry.values["op"], "custom_write");
}

#[metrics(subfield)]
struct BackendMetrics {
    latency_ms: u32,
}

#[metrics(tag(name = "operation"))]
enum WithFlatten {
    ReadStruct {
        #[metrics(flatten)]
        backend: BackendMetrics,
        bytes: usize,
    },
    ReadTuple(#[metrics(flatten)] BackendMetrics),
}

#[test]
fn tag_with_flatten() {
    let entry = test_metric(WithFlatten::ReadStruct {
        backend: BackendMetrics { latency_ms: 100 },
        bytes: 2048,
    });
    assert_eq!(entry.values["operation"], "ReadStruct");
    assert_eq!(entry.metrics["latency_ms"].as_u64(), 100);
    assert_eq!(entry.metrics["bytes"].as_u64(), 2048);

    let entry = test_metric(WithFlatten::ReadTuple(BackendMetrics { latency_ms: 50 }));
    assert_eq!(entry.values["operation"], "ReadTuple");
    assert_eq!(entry.metrics["latency_ms"].as_u64(), 50);
}

#[metrics(tag(name = "op"))]
enum TagWithFlattenPrefix {
    Read {
        #[metrics(flatten, prefix = "backend_")]
        backend: BackendMetrics,
    },
}

#[test]
fn tag_with_flatten_prefix() {
    let entry = test_metric(TagWithFlattenPrefix::Read {
        backend: BackendMetrics { latency_ms: 100 },
    });
    assert_eq!(entry.values["op"], "Read");
    assert_eq!(entry.metrics["backend_latency_ms"].as_u64(), 100);
}

#[derive(Entry)]
struct StatusEntry {
    code: u32,
}

#[metrics(tag(name = "result"))]
enum TagWithFlattenEntry {
    SuccessTuple(#[metrics(flatten_entry, no_close)] StatusEntry),
    SuccessStruct {
        #[metrics(flatten_entry, no_close)]
        status: StatusEntry,
        message: String,
    },
}

#[test]
fn tag_with_flatten_entry() {
    let entry = test_metric(TagWithFlattenEntry::SuccessTuple(StatusEntry { code: 200 }));
    assert_eq!(entry.values["result"], "SuccessTuple");
    assert_eq!(entry.metrics["code"].as_u64(), 200);

    let entry = test_metric(TagWithFlattenEntry::SuccessStruct {
        status: StatusEntry { code: 201 },
        message: "ok".to_string(),
    });
    assert_eq!(entry.values["result"], "SuccessStruct");
    assert_eq!(entry.metrics["code"].as_u64(), 201);
    assert_eq!(entry.values["message"], "ok");
}

#[metrics(subfield)]
struct RegionMetrics {
    #[metrics(sample_group)]
    region: &'static str,
    bytes: usize,
}

#[metrics(tag(name = "operation", sample_group))]
enum TagWithSampleGroup {
    ReadStruct {
        #[metrics(sample_group)]
        region: &'static str,
        bytes: usize,
    },
    ReadTuple(#[metrics(flatten)] RegionMetrics),
}

#[test]
fn tag_sample_group_with_field_sample_group() {
    let metric = TagWithSampleGroup::ReadStruct {
        region: "us-west-2",
        bytes: 2048,
    };
    let entry = RootEntry::new(metric.close());
    let sample_group: Vec<_> = entry.sample_group().collect();
    assert_eq!(sample_group.len(), 2);
    assert_eq!(sample_group[0].0, "operation");
    assert_eq!(sample_group[0].1, "ReadStruct");
    assert_eq!(sample_group[1].0, "region");
    assert_eq!(sample_group[1].1, "us-west-2");

    let metric = TagWithSampleGroup::ReadTuple(RegionMetrics {
        region: "eu-west-1",
        bytes: 4096,
    });
    let entry = RootEntry::new(metric.close());
    let sample_group: Vec<_> = entry.sample_group().collect();
    assert_eq!(sample_group.len(), 2);
    assert_eq!(sample_group[0].0, "operation");
    assert_eq!(sample_group[0].1, "ReadTuple");
    assert_eq!(sample_group[1].0, "region");
    assert_eq!(sample_group[1].1, "eu-west-1");
}
