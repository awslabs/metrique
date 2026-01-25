use metrique::{
    multi_flex::{FlexItem, MultiFlex},
    test_util::{TestEntrySink, test_entry_sink},
    unit_of_work::metrics,
};

#[metrics]
struct MyDevices {
    // devices.0.size etc.
    #[metrics(flatten, prefix = "devices")]
    devices: MultiFlex<Device>,
    top_level: usize,
}

#[metrics(subfield)]
struct Device {
    id: usize,
    size: usize,
}

impl FlexItem for Device {
    fn prefix_item(&self, idx: usize, mut buffer: impl std::fmt::Write) {
        write!(buffer, ".{idx}.").unwrap();
    }
}

#[test]
fn basic_test() {
    let mut devices = MyDevices {
        devices: Default::default(),
        top_level: 5,
    };

    devices.devices.push(Device { id: 1, size: 10 });
    devices.devices.push(Device { id: 2, size: 10000 });
    let TestEntrySink { sink, inspector } = test_entry_sink();
    drop(devices.append_on_drop(sink));
    let metrics = inspector.entries();
    dbg!(&metrics[0].metrics);
    assert_eq!(metrics[0].metrics["devices.0.size"], 10,);
    assert_eq!(metrics[0].metrics["devices.1.size"], 10000);
    assert_eq!(metrics[0].metrics["devices.0.id"], 1);
    assert_eq!(metrics[0].metrics["devices.1.id"], 2);
    assert_eq!(metrics[0].metrics["top_level"], 5);
}
