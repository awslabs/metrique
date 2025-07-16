// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{borrow::Cow, marker::PhantomData, sync::Arc};

use super::ValueWriter;

/// A trait for a function that formats a value in a custom way.
pub trait ValueFormatter<V: ?Sized> {
    /// Write `value` to `writer`
    fn format_value(writer: impl ValueWriter, value: &V);
}

impl<V: ?Sized, F> ValueFormatter<&V> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &&V) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

impl<V, F> ValueFormatter<Option<V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Option<V>) {
        if let Some(value) = value {
            <Self as ValueFormatter<V>>::format_value(writer, value)
        }
    }
}

impl<V: ?Sized, F> ValueFormatter<Box<V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Box<V>) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

impl<V: ?Sized, F> ValueFormatter<Arc<V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Arc<V>) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

impl<V: ToOwned + ?Sized, F> ValueFormatter<Cow<'_, V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Cow<V>) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

#[doc(hidden)]
/// A wrapper for a value that formats using a [ValueFormatter]
pub struct FormattedValue<'a, V, VF>(PhantomData<VF>, &'a V);

impl<'a, V, VF> FormattedValue<'a, V, VF> {
    #[doc(hidden)]
    pub fn new(value: &'a V) -> Self {
        Self(PhantomData, value)
    }
}

impl<V, VF> super::Value for FormattedValue<'_, V, VF>
where
    VF: ValueFormatter<V>,
{
    fn write(&self, writer: impl ValueWriter) {
        VF::format_value(writer, self.1);
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use std::{
        borrow::Cow,
        io,
        sync::Arc,
        time::{Duration, SystemTime},
    };

    use metrique_writer::{Entry, format::Format};
    use metrique_writer_format_emf::Emf;

    struct AsEpochSeconds;

    impl metrique_writer::value::ValueFormatter<SystemTime> for AsEpochSeconds {
        fn format_value(writer: impl metrique_writer::ValueWriter, value: &SystemTime) {
            let epoch_seconds = value
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            writer.string(&epoch_seconds.to_string());
        }
    }

    struct FormatString;
    impl metrique_writer::value::ValueFormatter<str> for FormatString {
        fn format_value(writer: impl metrique_writer::ValueWriter, value: &str) {
            writer.string(value);
        }
    }

    #[derive(Entry)]
    struct MyMetric {
        #[entry(timestamp)]
        timestamp: SystemTime,
        #[entry(format=AsEpochSeconds)]
        other_time: Option<Box<SystemTime>>,
        #[entry(format=AsEpochSeconds)]
        other_time_arc: Arc<SystemTime>,
        #[entry(format=AsEpochSeconds)]
        other_time_2: Option<Box<SystemTime>>,
        #[entry(format=FormatString)]
        cow: Cow<'static, str>,
    }

    #[test]
    fn test_format_mymetric() {
        let mut emf = Emf::no_validations("MyNS".into(), vec![vec![]]);
        let mut output = io::Cursor::new(vec![]);
        emf.format(
            &MyMetric {
                timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                other_time: Some(Box::new(SystemTime::UNIX_EPOCH + Duration::from_secs(2))),
                other_time_arc: Arc::new(SystemTime::UNIX_EPOCH + Duration::from_secs(3)),
                other_time_2: None,
                cow: Cow::Borrowed("string"),
            },
            &mut output,
        )
        .unwrap();
        let output: serde_json::Value = String::from_utf8(output.into_inner())
            .unwrap()
            .parse()
            .unwrap();
        assert_json_diff::assert_json_eq!(
            output,
            json!({
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "MyNS",
                            "Dimensions": [[]],
                            "Metrics": []
                        }
                    ],
                    "Timestamp": 1000
                },
                "other_time": "2",
                "other_time_arc": "3",
                "cow": "string",
            })
        );
    }
}
