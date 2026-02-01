use assert2::check;
use metrique::{OnParentDrop, Slot};
use metrique_macro::metrics;
use metrique_writer::sink::DevNullSink;

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Debug)]
struct Metrics {
    a: usize,
    #[metrics(flatten)]
    b: Slot<Nested>,
    #[metrics(flatten)]
    c: Slot<AnotherNested>,
}

#[metrics(subfield)]
#[derive(Default, Clone, Debug)]
struct Nested {
    inner_value: usize,
}

#[metrics(subfield)]
#[derive(Default, Clone, Debug)]
struct AnotherNested {
    another_value: usize,
}

#[tokio::test]
async fn debug_slot_closed() {
    let mut metrics = Metrics {
        a: 42,
        b: Slot::new(Nested { inner_value: 123 }),
        c: Slot::new(AnotherNested { another_value: 456 }),
    };

    check!(format!("{:?}", metrics.b) == "Slot { open: false, has_data: false, data: None }");
    let mut guard = metrics.b.open(OnParentDrop::Discard).unwrap();
    check!(format!("{:?}", metrics.b) == "Slot { open: true, has_data: false, data: None }");
    guard.inner_value = 1000;
    drop(guard);
    check!(format!("{:?}", metrics.b) == "Slot { open: true, has_data: true, data: None }");
    metrics.b.wait_for_data().await;
    check!(
        format!("{:?}", metrics.b)
            == "Slot { open: true, has_data: true, data: Some(NestedEntry { inner_value: 1000 }) }"
    );
}

#[test]
fn debug_slot_open() {
    let mut metrics = Metrics {
        a: 99,
        b: Slot::new(Nested { inner_value: 200 }),
        c: Slot::new(AnotherNested { another_value: 300 }),
    };

    // Open the slots - the guards now hold the data
    let _guard_b = metrics.b.open(OnParentDrop::Discard);
    let _guard_c = metrics.c.open(OnParentDrop::Discard);

    let debug_output = format!("{:?}", metrics);

    // When slots are open, open field is true
    check!(debug_output.contains("a: 99"));
    check!(debug_output.contains("open: true"));

    // Check the new Debug format
    check!(
        format!("{:?}", metrics.append_on_drop(DevNullSink::new()))
            == "AppendAndCloseOnDrop { value: Metrics { a: 99, b: Slot { open: true, has_data: false, data: None }, c: Slot { open: true, has_data: false, data: None } }, sink: DevNullSink }"
    );
}

#[test]
fn debug_slot_guard() {
    let mut metrics = Metrics {
        a: 1,
        b: Slot::new(Nested { inner_value: 777 }),
        c: Slot::new(AnotherNested { another_value: 888 }),
    };

    let guard_b = metrics.b.open(OnParentDrop::Discard).unwrap();

    check!(
        format!("{:?}", guard_b)
            == "SlotGuard { value: Nested { inner_value: 777 }, parent_is_closed: false, parent_drop_mode: Discard }"
    );
    drop(metrics);
    check!(
        format!("{:?}", guard_b)
            == "SlotGuard { value: Nested { inner_value: 777 }, parent_is_closed: true, parent_drop_mode: Discard }"
    );
}
