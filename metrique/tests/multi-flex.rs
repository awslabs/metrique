use metrique::{
    multi_flex::{FlexItem, MultiFlex},
    test_util::{TestEntrySink, test_entry_sink},
    unit_of_work::metrics,
};
use std::borrow::Cow;

#[metrics]
struct MyDevices {
    // devices.0.size etc.
    #[metrics(flatten, prefix = "devices")]
    devices: MultiFlex<Device>,
    top_level: usize,
}

#[metrics]
struct Device {
    id: String,
    size: usize,
}

impl FlexItem for Device {
    fn prefix_item(&self, idx: usize) -> std::borrow::Cow<'static, str> {
        return Cow::Owned(format!(".{idx}."));
    }
}

#[test]
fn basic_test() {
    let mut devices = MyDevices {
        devices: Default::default(),
        top_level: 5,
    };

    devices.devices.push(Device {
        id: "hello".into(),
        size: 10,
    });
    devices.devices.push(Device {
        id: "also hello".into(),
        size: 10000,
    });
    let TestEntrySink { sink, inspector } = test_entry_sink();
    drop(devices.append_on_drop(sink));
    let metrics = inspector.entries();
    assert_eq!(metrics[0].metrics["devices.0.size"], 10);
    assert_eq!(metrics[0].metrics["devices.1.size"], 10000);
    assert_eq!(metrics[0].values["devices.0.id"], "hello");
    assert_eq!(metrics[0].values["devices.1.id"], "also hello");
    assert_eq!(metrics[0].metrics["top_level"], 5);
}
