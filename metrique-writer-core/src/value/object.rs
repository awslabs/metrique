// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Rendering a struct field as a nested object rather than a flat scalar.
//!
//! [`ObjectValue`] describes a type whose fields render as object members, written through the
//! same [`EntryWriter`] an entry uses so any `#[metrics]` type nests. [`AsObject`] is the
//! [`ValueFormatter`] a field opts into with `#[metrics(format = AsObject)]`. Objects are not
//! metrics: timestamp, config, and per-member unit/dimension/flag metadata are ignored. A format
//! renders an object natively (JSON/EMF/LocalFormat, added later) or via the
//! [`ValueWriter::object`] default [`write_object_as_string`], which serializes to a JSON string
//! so the field is never dropped.
//!
//! The wrapper matrix is split for coherence: `Vec<O>`/`Option<O>`/`Option<Vec<O>>` are
//! [`AsObject`] formatter impls, while `&O`/`Box<O>`/`Arc<O>` forward on [`ObjectValue`] (`Box`
//! and `&` are `#[fundamental]`, so the equivalent `AsObject` impls would not be coherent). The
//! scalar and `Vec` impls stay disjoint only because no `Vec<_>` is [`ObjectValue`].

use std::sync::Arc;

use crate::descriptor::{FieldShape, ShapeRef};
use crate::json_encode::{push_json_string, push_observation};
use crate::{EntryWriter, Unit};

use super::{NotLifted, Observation, Value, ValueFormatter, ValueWriter};

/// A type whose fields can be emitted as members of a nested object.
///
/// Each `writer.value(name, v)` call inside [`write_object`](ObjectValue::write_object) becomes a
/// `"name": <v>` member. Distinct from [`Value`] on purpose: a bare `#[metrics]` struct field
/// stays a compile error unless it opts into object rendering. Carries no `Vec`/`Option` impls
/// (those are [`AsObject`] formatter impls) and forwards only through `&O`/`Box<O>`/`Arc<O>` — see
/// the module docs for why.
pub trait ObjectValue {
    /// Emit this object's members into the given writer.
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>);
}

impl<O: ObjectValue + ?Sized> ObjectValue for &O {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write_object(writer)
    }
}

impl<O: ObjectValue + ?Sized> ObjectValue for Box<O> {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write_object(writer)
    }
}

impl<O: ObjectValue + ?Sized> ObjectValue for Arc<O> {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        (**self).write_object(writer)
    }
}

/// A [`ValueFormatter`] that renders its value as a nested object by delegating to
/// [`ObjectValue::write_object`]. Opt in with `#[metrics(format = AsObject)]` (the derive
/// generates the [`ObjectValue`] impl). Handles `O`, `Option<O>`, `Vec<O>` (a JSON array), and
/// `Option<Vec<O>>`, plus those behind `&`/`Box`/`Arc`.
pub struct AsObject;

// Named so the container `SHAPE` consts below can borrow them.
const OBJECT_SHAPE: FieldShape<'static> = FieldShape::Object;
const OBJECT_LIST_SHAPE: FieldShape<'static> = FieldShape::List(ShapeRef::new(&OBJECT_SHAPE));

impl<O: ObjectValue + ?Sized> ValueFormatter<O, NotLifted> for AsObject {
    const SHAPE: FieldShape<'static> = FieldShape::Object;

    fn format_value(writer: impl ValueWriter, value: &O) {
        writer.object(value)
    }
}

// INVARIANT: this `Vec<O>` impl and the scalar-`O` impl above are disjoint only because no
// `Vec<_>` is ever `ObjectValue`. Adding a `Vec<O>: ObjectValue` impl overlaps them → E0119.
// (Module docs explain why the wrapper matrix is split across `ObjectValue` and `AsObject`.)
impl<O: ObjectValue> ValueFormatter<Vec<O>, NotLifted> for AsObject {
    const SHAPE: FieldShape<'static> = FieldShape::List(ShapeRef::new(&OBJECT_SHAPE));

    fn format_value(writer: impl ValueWriter, value: &Vec<O>) {
        writer.values(ObjectRef::wrap_slice(value).iter());
    }
}

impl<O: ObjectValue> ValueFormatter<Option<O>, NotLifted> for AsObject {
    const SHAPE: FieldShape<'static> = FieldShape::Optional(ShapeRef::new(&OBJECT_SHAPE));

    fn format_value(writer: impl ValueWriter, value: &Option<O>) {
        // A `None` field is omitted entirely, distinct from an all-empty object, which writes `{}`.
        if let Some(value) = value {
            writer.object(value)
        }
    }
}

impl<O: ObjectValue> ValueFormatter<Option<Vec<O>>, NotLifted> for AsObject {
    const SHAPE: FieldShape<'static> = FieldShape::Optional(ShapeRef::new(&OBJECT_LIST_SHAPE));

    fn format_value(writer: impl ValueWriter, value: &Option<Vec<O>>) {
        // An absent field is omitted; a present one renders as an array, empty Vec included (`[]`).
        if let Some(value) = value {
            writer.values(ObjectRef::wrap_slice(value).iter());
        }
    }
}

/// Bridges an [`ObjectValue`] into the existing [`ValueWriter::values`] array path by giving it a
/// [`Value`] impl that calls [`ValueWriter::object`].
///
/// It is `#[repr(transparent)]` so a `&[O]` can be reinterpreted as `&[ObjectRef<O>]`, letting the
/// [`AsObject`] `Vec` formatter hand the elements straight to `values()` with no intermediate
/// allocation. `ObjectRef` sets `SHAPE = Object`, which
/// [`write_values_as_string`](super::write_values_as_string) reads to bracket object arrays into a
/// JSON array on formats without native support. Internal to the library;
/// constructed by [`AsObject`] and the tests, never by users.
#[repr(transparent)]
pub(crate) struct ObjectRef<O: ?Sized>(pub(crate) O);

impl<O> ObjectRef<O> {
    /// Reinterpret a slice of objects as a slice of `ObjectRef`.
    pub(crate) fn wrap_slice(slice: &[O]) -> &[ObjectRef<O>] {
        // SAFETY: `ObjectRef<O>` is `#[repr(transparent)]` over `O`, so it has identical size and
        // alignment, and `[ObjectRef<O>]` has the same layout as `[O]`, including slice length
        // metadata. The cast adds no fields and reinterprets in place.
        unsafe { &*(slice as *const [O] as *const [ObjectRef<O>]) }
    }
}

impl<O: ObjectValue + ?Sized> Value for ObjectRef<O> {
    const SHAPE: FieldShape<'static> = FieldShape::Object;
    const UNIT: Unit = Unit::None;

    fn write(&self, writer: impl ValueWriter) {
        writer.object(&self.0)
    }
}

/// The default [`ValueWriter::object`] behaviour: serialize the object to a JSON string via
/// [`ValueWriter::string`], the object analogue of
/// [`write_values_as_string`](super::write_values_as_string). Nested objects and arrays are
/// encoded natively, not double-escaped. Wrapper writers should forward `object` rather than call
/// this, or native object support is bypassed.
///
/// A member that writes nothing (e.g. `None` or an empty distribution) is omitted rather than
/// emitted as `"name":null`. A value that reports a [`ValidationError`](crate::ValidationError) is
/// also omitted: a string serializer has no channel to surface it.
pub fn write_object_as_string<O: ObjectValue + ?Sized>(writer: impl ValueWriter, object: &O) {
    let mut buf = String::new();
    write_json_object(&mut buf, object);
    writer.string(&buf);
}

/// Render an object body as a JSON `{...}` into `buf`, sharing the canonical scalar encoders in
/// [`crate::json_encode`] so a member renders identically here and in the native JSON format.
fn write_json_object<O: ObjectValue + ?Sized>(buf: &mut String, object: &O) {
    buf.push('{');
    let mut member_writer = JsonCaptureEntryWriter {
        buf,
        wrote_member: false,
    };
    object.write_object(&mut member_writer);
    buf.push('}');
}

/// Builds `"name":<value>` members into a JSON string buffer.
struct JsonCaptureEntryWriter<'a> {
    buf: &'a mut String,
    wrote_member: bool,
}

impl<'a> EntryWriter<'a> for JsonCaptureEntryWriter<'_> {
    fn timestamp(&mut self, _timestamp: std::time::SystemTime) {}

    fn value(&mut self, name: impl Into<std::borrow::Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        // Record the position before the separator and name so a member that writes nothing can be
        // rolled back, leaving no dangling `,"name":`.
        let recorded_len = self.buf.len();
        if self.wrote_member {
            self.buf.push(',');
        }
        let _ = push_json_string(self.buf, &name.into());
        self.buf.push(':');
        let value_start = self.buf.len();
        value.write(JsonCaptureValueWriter(self.buf));
        if self.buf.len() == value_start {
            self.buf.truncate(recorded_len);
        } else {
            self.wrote_member = true;
        }
    }

    fn config(&mut self, _config: &'a dyn crate::entry::EntryConfig) {}
}

/// Renders a member value as bare JSON (metric metadata discarded). Nested objects and arrays
/// recurse natively, not via `string()`, so they are not double-escaped.
struct JsonCaptureValueWriter<'a>(&'a mut String);

impl ValueWriter for JsonCaptureValueWriter<'_> {
    fn string(self, value: &str) {
        let _ = push_json_string(self.0, value);
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        _unit: Unit,
        _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: super::MetricFlags<'_>,
    ) {
        let buf = self.0;
        let mut iter = distribution.into_iter();
        let Some(first) = iter.next() else { return };
        match iter.next() {
            None => {
                let _ = push_observation(buf, first, None);
            }
            Some(second) => {
                buf.push('[');
                let _ = push_observation(buf, first, None);
                buf.push(',');
                let _ = push_observation(buf, second, None);
                for obs in iter {
                    buf.push(',');
                    let _ = push_observation(buf, obs, None);
                }
                buf.push(']');
            }
        }
    }

    fn error(self, _error: crate::ValidationError) {}

    fn object<O: ObjectValue + ?Sized>(self, object: &O) {
        write_json_object(self.0, object)
    }

    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        let buf = self.0;
        buf.push('[');
        let mut wrote_any = false;
        for value in values {
            let before = buf.len();
            if wrote_any {
                buf.push(',');
            }
            let after_sep = buf.len();
            value.write(JsonCaptureValueWriter(buf));
            if buf.len() > after_sep {
                wrote_any = true;
            } else {
                buf.truncate(before);
            }
        }
        buf.push(']');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::FieldShape;

    /// Minimal `ValueWriter` that captures the single `string()` call a format without native
    /// object support receives.
    struct CaptureString<'a>(&'a mut Option<String>);

    impl ValueWriter for CaptureString<'_> {
        fn string(self, value: &str) {
            *self.0 = Some(value.to_owned());
        }

        fn metric<'a>(
            self,
            _distribution: impl IntoIterator<Item = Observation>,
            _unit: Unit,
            _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
            _flags: crate::MetricFlags<'_>,
        ) {
            unreachable!("objects never reach metric()")
        }

        fn error(self, _error: crate::ValidationError) {
            unreachable!("objects never reach error()")
        }
    }

    /// Renders `value` through the default (non-overridden) `object()` implementation.
    fn fallback_string<V: Value>(value: &V) -> String {
        let mut captured = None;
        value.write(CaptureString(&mut captured));
        captured.expect("expected a string fallback")
    }

    /// Renders `values` through the default (non-overridden) `values()` implementation.
    fn fallback_values_string<'a, V: Value + 'a>(
        values: impl IntoIterator<Item = &'a V>,
    ) -> String {
        let mut captured = None;
        crate::value::write_values_as_string(CaptureString(&mut captured), values);
        captured.expect("expected a string fallback")
    }

    struct Endpoint {
        host: &'static str,
        port: u64,
    }

    impl ObjectValue for Endpoint {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Host", &self.host);
            writer.value("Port", &self.port);
        }
    }

    #[test]
    fn object_falls_back_to_json_string() {
        let endpoint = Endpoint {
            host: "example.com",
            port: 443,
        };
        assert_eq!(
            fallback_string(&ObjectRef(endpoint)),
            r#"{"Host":"example.com","Port":443}"#
        );
    }

    #[test]
    fn empty_object_falls_back_to_empty_braces() {
        struct Empty;
        impl ObjectValue for Empty {
            fn write_object<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {}
        }
        assert_eq!(fallback_string(&ObjectRef(Empty)), "{}");
    }

    struct Outer {
        name: &'static str,
        inner: Endpoint,
        retries: Vec<u64>,
        absent: Option<u64>,
    }

    impl ObjectValue for Outer {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Name", &self.name);
            writer.value("Inner", &ObjectRef(&self.inner));
            writer.value("Retries", &self.retries);
            writer.value("Absent", &self.absent);
        }
    }

    #[test]
    fn nested_objects_and_arrays_survive_the_fallback_without_double_encoding() {
        let outer = Outer {
            name: "a\"b\nc",
            inner: Endpoint {
                host: "example.com",
                port: 443,
            },
            retries: vec![1, 2, 3],
            absent: None,
        };
        let json = fallback_string(&ObjectRef(outer));
        // The `Absent` (None) member is omitted entirely, not rendered as `"Absent":null`.
        assert_eq!(
            json,
            r#"{"Name":"a\"b\nc","Inner":{"Host":"example.com","Port":443},"Retries":[1,2,3]}"#
        );
        // One `JSON.parse` recovers the whole tree (nested object/array are native, not escaped).
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["Inner"]["Host"], "example.com");
        assert_eq!(parsed["Retries"], serde_json::json!([1, 2, 3]));
        assert!(parsed.get("Absent").is_none());
    }

    #[test]
    fn none_member_is_omitted_in_first_middle_last_and_only_positions() {
        struct Members {
            a: Option<u64>,
            b: Option<u64>,
            c: Option<u64>,
        }
        impl ObjectValue for Members {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("A", &self.a);
                writer.value("B", &self.b);
                writer.value("C", &self.c);
            }
        }
        let js = |m| fallback_string(&ObjectRef(m));
        // first absent
        assert_eq!(
            js(Members {
                a: None,
                b: Some(2),
                c: Some(3)
            }),
            r#"{"B":2,"C":3}"#
        );
        // middle absent
        assert_eq!(
            js(Members {
                a: Some(1),
                b: None,
                c: Some(3)
            }),
            r#"{"A":1,"C":3}"#
        );
        // last absent
        assert_eq!(
            js(Members {
                a: Some(1),
                b: Some(2),
                c: None
            }),
            r#"{"A":1,"B":2}"#
        );
        // all absent → empty object, not `{,}`
        assert_eq!(
            js(Members {
                a: None,
                b: None,
                c: None
            }),
            "{}"
        );
    }

    #[test]
    fn repeated_observation_renders_total_and_count() {
        struct HasRepeated;
        impl ObjectValue for HasRepeated {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value(
                    "Latency",
                    &Observation::Repeated {
                        total: 150.0,
                        occurrences: 3,
                    },
                );
            }
        }
        assert_eq!(
            fallback_string(&ObjectRef(HasRepeated)),
            r#"{"Latency":{"total":150,"count":3}}"#
        );
    }

    /// An array of objects must degrade to a bracketed JSON array so one `JSON.parse` recovers it;
    /// without brackets the field text would be `{..},{..}`, which is not a JSON value.
    #[test]
    fn object_array_falls_back_to_bracketed_json_array() {
        let endpoints = [
            Endpoint {
                host: "a.example.com",
                port: 1,
            },
            Endpoint {
                host: "b.example.com",
                port: 2,
            },
        ];
        assert_eq!(
            fallback_values_string(ObjectRef::wrap_slice(&endpoints)),
            r#"[{"Host":"a.example.com","Port":1},{"Host":"b.example.com","Port":2}]"#
        );
    }

    #[test]
    fn empty_object_array_falls_back_to_empty_json_array() {
        let empty: &[Endpoint] = &[];
        assert_eq!(fallback_values_string(ObjectRef::wrap_slice(empty)), "[]");
    }

    /// Bracketing is keyed off `V::SHAPE == FieldShape::Object`, so arrays of non-object elements
    /// keep the historical bare comma-joined form.
    #[test]
    fn non_object_array_fallback_is_not_bracketed() {
        assert_eq!(fallback_values_string(&[1u64, 2, 3]), "1,2,3");
        assert_eq!(fallback_values_string(&["a", "b"]), "a,b");
        assert_eq!(fallback_values_string::<u64>(&[]), "");
    }

    #[test]
    fn object_ref_reports_object_shape() {
        assert_eq!(<ObjectRef<Endpoint> as Value>::SHAPE, FieldShape::Object);
    }

    // A `Phase { children: Vec<Phase> }` tree exercises the `values()` → `object()` recursion
    // (an object-array member nested inside an object body), which the plain-`Vec<scalar>` test
    // above does not. This is the recursion requirement.
    #[test]
    fn nested_object_arrays_recurse_natively_through_the_fallback() {
        struct Phase {
            name: &'static str,
            children: Vec<Phase>,
        }
        impl ObjectValue for Phase {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Name", &self.name);
                writer.value("Children", ObjectRef::wrap_slice(&self.children));
            }
        }

        let tree = Phase {
            name: "root",
            children: vec![
                Phase {
                    name: "a",
                    children: vec![],
                },
                Phase {
                    name: "b",
                    children: vec![Phase {
                        name: "b1",
                        children: vec![],
                    }],
                },
            ],
        };
        let json = fallback_string(&ObjectRef(tree));
        assert_eq!(
            json,
            r#"{"Name":"root","Children":[{"Name":"a","Children":[]},{"Name":"b","Children":[{"Name":"b1","Children":[]}]}]}"#
        );
        // One `JSON.parse` recovers the tree two levels deep.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["Children"][1]["Children"][0]["Name"], "b1");
    }

    #[test]
    fn multi_observation_member_is_a_json_array_and_empty_distribution_is_omitted() {
        struct MultiObs(Vec<u64>);
        impl Value for MultiObs {
            fn write(&self, writer: impl ValueWriter) {
                writer.metric(
                    self.0.iter().map(|&v| Observation::Unsigned(v)),
                    Unit::None,
                    [],
                    crate::MetricFlags::empty(),
                );
            }
        }
        struct Metrics {
            many: MultiObs,
            empty: MultiObs,
        }
        impl ObjectValue for Metrics {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Many", &self.many);
                // An empty distribution writes nothing, so the member is omitted.
                writer.value("Empty", &self.empty);
            }
        }
        assert_eq!(
            fallback_string(&ObjectRef(Metrics {
                many: MultiObs(vec![1, 2, 3]),
                empty: MultiObs(vec![]),
            })),
            r#"{"Many":[1,2,3]}"#
        );
    }

    #[test]
    fn box_and_arc_forward_to_the_inner_object() {
        let expected = r#"{"Host":"example.com","Port":443}"#;
        let make = || Endpoint {
            host: "example.com",
            port: 443,
        };
        assert_eq!(fallback_string(&ObjectRef(Box::new(make()))), expected);
        assert_eq!(fallback_string(&ObjectRef(Arc::new(make()))), expected);
    }

    /// Render a value through `AsObject` (the non-overridden `object`/`values` fallback), or
    /// `None` if the formatter wrote nothing.
    fn as_object<V>(value: &V) -> Option<String>
    where
        AsObject: ValueFormatter<V, NotLifted>,
    {
        let mut captured = None;
        <AsObject as ValueFormatter<V, NotLifted>>::format_value(
            CaptureString(&mut captured),
            value,
        );
        captured
    }

    #[test]
    fn as_object_shapes() {
        assert_eq!(
            <AsObject as ValueFormatter<Endpoint, NotLifted>>::SHAPE,
            FieldShape::Object
        );
        assert!(matches!(
            <AsObject as ValueFormatter<Vec<Endpoint>, NotLifted>>::SHAPE,
            FieldShape::List(inner) if *inner.get() == FieldShape::Object
        ));
        assert!(matches!(
            <AsObject as ValueFormatter<Option<Endpoint>, NotLifted>>::SHAPE,
            FieldShape::Optional(inner) if *inner.get() == FieldShape::Object
        ));
        assert!(matches!(
            <AsObject as ValueFormatter<Option<Vec<Endpoint>>, NotLifted>>::SHAPE,
            FieldShape::Optional(outer)
                if matches!(*outer.get(), FieldShape::List(inner) if *inner.get() == FieldShape::Object)
        ));
    }

    #[test]
    fn as_object_render_matrix() {
        let ep = || Endpoint { host: "h", port: 1 };
        let one = r#"{"Host":"h","Port":1}"#;

        // Single object, and behind each pointer wrapper.
        assert_eq!(as_object(&ep()).as_deref(), Some(one));
        assert_eq!(as_object(&Box::new(ep())).as_deref(), Some(one));
        assert_eq!(as_object(&Arc::new(ep())).as_deref(), Some(one));

        // Option<O>: Some renders, None writes nothing (key omitted).
        assert_eq!(as_object(&Some(ep())).as_deref(), Some(one));
        assert_eq!(as_object(&None::<Endpoint>), None);

        // Vec<O>: array of objects; empty Vec is `[]` (present, not omitted).
        assert_eq!(
            as_object(&vec![ep(), ep()]).as_deref(),
            Some(r#"[{"Host":"h","Port":1},{"Host":"h","Port":1}]"#)
        );
        assert_eq!(as_object(&Vec::<Endpoint>::new()).as_deref(), Some("[]"));

        // Option<Vec<O>>: present renders the array (incl. empty), absent writes nothing.
        assert_eq!(
            as_object(&Some(vec![ep()])).as_deref(),
            Some(r#"[{"Host":"h","Port":1}]"#)
        );
        assert_eq!(
            as_object(&Some(Vec::<Endpoint>::new())).as_deref(),
            Some("[]")
        );
        assert_eq!(as_object(&None::<Vec<Endpoint>>), None);
    }
}
