// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{borrow::Cow, fmt::Display, marker::PhantomData, sync::Arc};

use super::ValueWriter;

mod private {
    pub trait Sealed {}
}

pub trait Liftability: private::Sealed {}

/// X
pub struct Lifted;
/// X
pub struct NotLifted;

impl private::Sealed for Lifted {}
impl private::Sealed for NotLifted {}
impl Liftability for Lifted {}
impl Liftability for NotLifted {}

/// A trait for a function that formats a value in a custom way. Used for
/// `#[derive(Entry)]`'s `#[entry(format=FORMATTER)]` and `#[metrics]`'s
/// `#[metrics(format=FORMATTER)]`.
///
/// If the liftability is [`Lifted`] (the default), this [`ValueFormatter`] will be lifted over
/// various types such as [`Arc`] and [`Option`] (so you can format e.g. `Option<Arc<T>>`),
/// for example:
///
/// ```
/// # use std::sync::Arc;
/// # use std::time::SystemTime;
/// # use metrique_writer::Entry;
/// struct AsEpochSeconds;
/// impl metrique_writer::value::ValueFormatter<SystemTime> for AsEpochSeconds {
///     fn format_value(writer: impl metrique_writer::ValueWriter, value: &SystemTime) {
///         let epoch_seconds = value
///             .duration_since(SystemTime::UNIX_EPOCH)
///             .unwrap()
///             .as_secs();
///         writer.string(&epoch_seconds.to_string());
///     }
/// }
///
/// #[derive(Entry)]
/// struct MyEntry {
///     #[entry(format=AsEpochSeconds)]
///     timestamp: Option<Arc<SystemTime>>, // lifting over Arc and Option
/// }
/// ```
///
/// If liftability would cause a coherence error (if the impl is a blanket impl),
/// you could implement [`ValueFormatter`] with [`NotLifted`], and that
/// can be similarly used in the macro, for example:
///
/// ```
/// # use metrique_writer::{Entry, ValueWriter, value::{NotLifted, ValueFormatter}};
/// # use std::fmt::Display;
/// pub struct ToString;
///
/// impl<T: Display + ?Sized> ValueFormatter<T, NotLifted> for ToString {
///     fn format_value(writer: impl ValueWriter, value: &T) {
///         writer.string(&value.to_string());
///     }
/// }
///
/// #[derive(Entry)]
/// struct MyMetric {
///     #[entry(format = ToString)]
///     my_field: bool, // formats as a string, "true" or "false".
/// }
/// ```
pub trait ValueFormatter<V: ?Sized, L: Liftability = Lifted> {
    /// Write `value` to `writer`
    fn format_value(writer: impl ValueWriter, value: &V);
}

/// A `ValueFormatter` for values that implement [Display] that formats them as a string.
///
/// Example:
///
/// ```
/// # use metrique_writer::Entry;
/// # use metrique_writer::value::ToString;
/// #[derive(Entry)]
/// struct MyMetric {
///     #[entry(format = ToString)]
///     my_field: bool, // formats as a string, "true" or "false".
/// }
/// ```
pub struct ToString;

impl<T: Display + ?Sized> ValueFormatter<T, NotLifted> for ToString {
    fn format_value(writer: impl ValueWriter, value: &T) {
        writer.string(&value.to_string());
    }
}

impl<V: ?Sized, F: ?Sized> ValueFormatter<&V> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &&V) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

impl<V, F: ?Sized> ValueFormatter<Option<V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Option<V>) {
        if let Some(value) = value {
            <Self as ValueFormatter<V>>::format_value(writer, value)
        }
    }
}

impl<V: ?Sized, F: ?Sized> ValueFormatter<Box<V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Box<V>) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

impl<V: ?Sized, F: ?Sized> ValueFormatter<Arc<V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Arc<V>) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

impl<V: ToOwned + ?Sized, F: ?Sized> ValueFormatter<Cow<'_, V>> for F
where
    F: ValueFormatter<V>,
{
    fn format_value(writer: impl ValueWriter, value: &Cow<V>) {
        <Self as ValueFormatter<V>>::format_value(writer, value)
    }
}

#[doc(hidden)]
/// A wrapper for a value that formats using a [ValueFormatter]
pub struct FormattedValue<'a, V, VF, L = Lifted>(PhantomData<(VF, L)>, &'a V);

impl<'a, V, VF, L> FormattedValue<'a, V, VF, L> {
    #[doc(hidden)]
    pub fn new(value: &'a V) -> Self {
        Self(PhantomData, value)
    }
}

impl<V, VF, L: Liftability> super::Value for FormattedValue<'_, V, VF, L>
where
    VF: ValueFormatter<V, L>,
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
