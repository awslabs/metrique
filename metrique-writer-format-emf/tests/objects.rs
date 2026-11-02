// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Native object/object-array rendering in EMF. Objects are not metrics, so the object body
//! renders as plain JSON (the same core encoder the JSON format uses), not EMF's metric
//! conventions: a `Repeated` member is `{"total":…,"count":…}` and a NaN member is `null`.

use std::borrow::Cow;
use std::time::{Duration, SystemTime};

use assert_json_diff::assert_json_eq;
use metrique_writer::value::WithDimension;
use metrique_writer::{Entry, EntryIoStream, EntryWriter, FormatExt, IoStreamError};
use metrique_writer_core::descriptor::{FieldShape, KnownShape};
use metrique_writer_core::test_stream::TestSink;
use metrique_writer_core::unit::NegativeScale;
use metrique_writer_core::value::{ObjectValue, Value, ValueWriter};
use metrique_writer_core::{MetricFlags, Observation, Unit, ValidationError};
use metrique_writer_format_emf::{Emf, EntryDimensions, HighStorageResolution};
use rstest::rstest;
use serde_json::json;

/// A `Value` that renders a single `ObjectValue` through `object()`.
struct DirectObject<O>(O);

impl<O: ObjectValue> Value for DirectObject<O> {
    const SHAPE: FieldShape<'static> = FieldShape::Object;
    const UNIT: Unit = Unit::None;

    fn write(&self, writer: impl ValueWriter) {
        writer.object(&self.0);
    }
}

/// An entry with a fixed timestamp and a single field, so output is deterministic.
struct Field<V>(&'static str, V);

impl<V: Value + Send + 'static> Entry for Field<V> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        writer.value(self.0, &self.1);
    }
}

/// Format one entry with a single dimension set made of `dimensions`.
fn emit(dimensions: &[&str], entry: &impl Entry) -> Result<String, IoStreamError> {
    let sink = TestSink::default();
    let dimension_sets = vec![dimensions.iter().map(|d| d.to_string()).collect()];
    let mut stream = Emf::all_validations("App".into(), dimension_sets).output_to(sink.clone());
    stream.next(entry)?;
    Ok(sink.dump())
}

/// Render the entry built by `make`, asserting the boxed path matches the unboxed path
/// byte-for-byte.
fn render_checked<E: Entry + Send + 'static>(make: impl Fn() -> E) -> serde_json::Value {
    let unboxed = emit(&[], &make()).unwrap();
    let boxed = emit(&[], &make().boxed()).unwrap();
    assert_eq!(unboxed, boxed, "boxed output must match unboxed output");
    serde_json::from_str(&unboxed).unwrap()
}

/// Format the entry built by `make` expecting a validation failure, asserting the boxed path
/// fails identically. Returns the error message.
fn render_err_checked<E: Entry + Send + 'static>(
    dimensions: &[&str],
    make: impl Fn() -> E,
) -> String {
    let unboxed = emit(dimensions, &make()).unwrap_err().to_string();
    let boxed = emit(dimensions, &make().boxed()).unwrap_err().to_string();
    assert_eq!(unboxed, boxed, "boxed error must match unboxed error");
    unboxed
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

fn ep() -> Endpoint {
    Endpoint {
        host: "example.com",
        port: 443,
    }
}

/// An object field lands natively in the properties section, and it is not declared as a
/// CloudWatch metric (objects are not metrics).
#[test]
fn single_object_renders_natively() {
    let doc = render_checked(|| Field("Endpoint", DirectObject(ep())));
    assert_json_eq!(doc["Endpoint"], json!({"Host": "example.com", "Port": 443}));
    assert_json_eq!(doc["_aws"]["CloudWatchMetrics"][0]["Metrics"], json!([]));
}

/// An object field coexists with a string field and a metric; only the metric is declared.
#[test]
fn object_alongside_string_and_metric() {
    struct Mixed;
    impl Entry for Mixed {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
            writer.value("Operation", &"Get");
            writer.value("Latency", &5u64);
            writer.value("Endpoint", &DirectObject(ep()));
        }
    }
    let doc = render_checked(|| Mixed);
    assert_json_eq!(doc["Operation"], json!("Get"));
    assert_json_eq!(doc["Latency"], json!(5));
    assert_json_eq!(doc["Endpoint"], json!({"Host": "example.com", "Port": 443}));
    assert_json_eq!(
        doc["_aws"]["CloudWatchMetrics"][0]["Metrics"],
        json!([{"Name": "Latency"}])
    );
}

/// Two elements stay inline in the boxed path's small-vector buffer; nine spill past
/// `VALUES_INLINE_CAPACITY` (8) onto the heap.
#[rstest]
#[case::inline(2)]
#[case::spilled(9)]
fn array_of_objects_renders_as_json_array(#[case] len: u64) {
    let doc = render_checked(|| {
        Field(
            "Endpoints",
            (0..len)
                .map(|port| DirectObject(Endpoint { host: "h", port }))
                .collect::<Vec<_>>(),
        )
    });
    let expected: Vec<_> = (0..len)
        .map(|port| json!({"Host": "h", "Port": port}))
        .collect();
    assert_json_eq!(doc["Endpoints"], json!(expected));
}

#[test]
fn empty_object_array_renders_empty_array() {
    let doc = render_checked(|| Field("E", Vec::<DirectObject<Endpoint>>::new()));
    assert_json_eq!(doc["E"], json!([]));
}

#[test]
fn all_empty_object_renders_empty_braces() {
    struct Empty;
    impl ObjectValue for Empty {
        fn write_object<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {}
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Empty)));
    assert_json_eq!(doc["Obj"], json!({}));
}

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

/// A `None` member is omitted with no dangling `"Key":` and no stray comma — the member rollback.
#[rstest]
#[case::first_absent(None, Some(2), Some(3), json!({"B": 2, "C": 3}))]
#[case::middle_absent(Some(1), None, Some(3), json!({"A": 1, "C": 3}))]
#[case::last_absent(Some(1), Some(2), None, json!({"A": 1, "B": 2}))]
#[case::only_middle_present(None, Some(2), None, json!({"B": 2}))]
#[case::all_absent(None, None, None, json!({}))]
fn none_members_are_omitted(
    #[case] a: Option<u64>,
    #[case] b: Option<u64>,
    #[case] c: Option<u64>,
    #[case] expected: serde_json::Value,
) {
    let doc = render_checked(|| Field("Obj", DirectObject(Members { a, b, c })));
    assert_json_eq!(doc["Obj"], expected);
}

struct Phase {
    name: &'static str,
    children: Vec<Phase>,
}

impl ObjectValue for Phase {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Name", &self.name);
        writer.value(
            "Children",
            &self.children.iter().map(DirectObject).collect::<Vec<_>>(),
        );
    }
}

#[test]
fn nested_object_tree_renders_natively_to_full_depth() {
    let doc = render_checked(|| {
        Field(
            "Root",
            DirectObject(Phase {
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
            }),
        )
    });
    assert_json_eq!(
        doc["Root"],
        json!({
            "Name": "root",
            "Children": [
                {"Name": "a", "Children": []},
                {"Name": "b", "Children": [{"Name": "b1", "Children": []}]}
            ]
        })
    );
}

/// A dimensioned member renders as its bare value, dimension dropped.
#[test]
fn object_member_dimensions_are_dropped() {
    struct Obj;
    impl ObjectValue for Obj {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Count", &WithDimension::new(10u64, "Region", "us-east-1"));
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Obj)));
    assert_json_eq!(doc["Obj"], json!({"Count": 10}));
}

/// A member carrying a unit renders as its bare value, unit dropped.
#[test]
fn object_member_units_are_dropped() {
    struct Latency;
    impl Value for Latency {
        const SHAPE: FieldShape<'static> = FieldShape::Known(KnownShape::U64);
        const UNIT: Unit = Unit::None;

        fn write(&self, writer: impl ValueWriter) {
            writer.metric(
                [Observation::Unsigned(5)],
                Unit::Second(NegativeScale::Milli),
                [],
                MetricFlags::empty(),
            );
        }
    }
    struct Obj;
    impl ObjectValue for Obj {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Latency", &Latency);
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Obj)));
    assert_json_eq!(doc["Obj"], json!({"Latency": 5}));
}

/// A member carrying an EMF flag renders as its bare value: no `StorageResolution` is emitted
/// and the object is still not declared as a metric.
#[test]
fn object_member_flags_are_dropped() {
    struct Obj;
    impl ObjectValue for Obj {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Count", &HighStorageResolution::from(7u64));
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Obj)));
    assert_json_eq!(doc["Obj"], json!({"Count": 7}));
    assert_json_eq!(doc["_aws"]["CloudWatchMetrics"][0]["Metrics"], json!([]));
}

/// A `Value` writing an arbitrary distribution.
struct Dist(Vec<Observation>);

impl Value for Dist {
    const SHAPE: FieldShape<'static> = FieldShape::Known(KnownShape::F64);
    const UNIT: Unit = Unit::None;

    fn write(&self, writer: impl ValueWriter) {
        writer.metric(self.0.iter().copied(), Unit::None, [], MetricFlags::empty());
    }
}

struct DistObject(Dist);

impl ObjectValue for DistObject {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Keep", &1u64);
        writer.value("Samples", &self.0);
    }
}

/// A member with multiple observations renders as a JSON array. Objects render as plain JSON (not
/// EMF's metric encoding), so a NaN observation becomes `null` rather than being dropped.
#[rstest]
#[case::all_finite(
    vec![Observation::Unsigned(1), Observation::Unsigned(2), Observation::Unsigned(3)],
    json!({"Keep": 1, "Samples": [1, 2, 3]}),
)]
#[case::nan_becomes_null(
    vec![Observation::Unsigned(1), Observation::Floating(f64::NAN), Observation::Unsigned(3)],
    json!({"Keep": 1, "Samples": [1, null, 3]}),
)]
#[case::all_nan(
    vec![Observation::Floating(f64::NAN), Observation::Floating(f64::NAN)],
    json!({"Keep": 1, "Samples": [null, null]}),
)]
#[case::empty_distribution_omitted(vec![], json!({"Keep": 1}))]
fn object_member_distribution(
    #[case] observations: Vec<Observation>,
    #[case] expected: serde_json::Value,
) {
    let doc = render_checked(|| Field("Obj", DirectObject(DistObject(Dist(observations.clone())))));
    assert_json_eq!(doc["Obj"], expected);
}

/// Objects render as plain JSON, so a `Repeated` member renders as `{"total":…,"count":…}` — the
/// same as the JSON format, not EMF's metric mean.
#[test]
fn object_member_repeated_renders_as_total_and_count() {
    struct Latency;
    impl ObjectValue for Latency {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value(
                "Dist",
                &Observation::Repeated {
                    total: 150.0,
                    occurrences: 3,
                },
            );
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Latency)));
    assert_json_eq!(doc["Obj"], json!({"Dist": {"total": 150, "count": 3}}));
}

struct FloatMember(f64);

impl ObjectValue for FloatMember {
    fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Keep", &1u64);
        writer.value("X", &self.0);
    }
}

/// A NaN member renders as `null` (plain-JSON semantics, same as the JSON format) — EMF's metric
/// NaN-drop does not apply inside an object, because objects are not metrics.
#[test]
fn object_member_nan_renders_as_null() {
    let doc = render_checked(|| Field("Obj", DirectObject(FloatMember(f64::NAN))));
    assert_json_eq!(doc["Obj"], json!({"Keep": 1, "X": null}));
}

/// An infinite member is clamped to the nearest finite `f64` and written, not omitted.
#[rstest]
#[case::positive(f64::INFINITY, f64::MAX)]
#[case::negative(f64::NEG_INFINITY, -f64::MAX)]
fn object_member_infinity_is_clamped(#[case] value: f64, #[case] expected: f64) {
    let doc = render_checked(|| Field("Obj", DirectObject(FloatMember(value))));
    assert_eq!(doc["Obj"]["X"].as_f64(), Some(expected));
}

/// A member whose value reports a `ValidationError` is omitted and the entry still formats:
/// objects are not metrics, so a member error does not fail the entry (matching the JSON format).
#[test]
fn object_member_validation_error_is_omitted() {
    struct Invalid;
    impl Value for Invalid {
        const SHAPE: FieldShape<'static> = FieldShape::Known(KnownShape::U64);
        const UNIT: Unit = Unit::None;

        fn write(&self, writer: impl ValueWriter) {
            writer.error(ValidationError::invalid("bad member"));
        }
    }
    struct Obj;
    impl ObjectValue for Obj {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Keep", &1u64);
            writer.value("Bad", &Invalid);
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Obj)));
    assert_json_eq!(doc["Obj"], json!({"Keep": 1}));
}

/// String members are JSON-escaped and round-trip unchanged.
#[test]
fn object_member_strings_are_escaped() {
    struct Weird;
    impl ObjectValue for Weird {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("Msg", &"a\"b\nc");
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Weird)));
    assert_json_eq!(doc["Obj"]["Msg"], json!("a\"b\nc"));
}

/// `timestamp` and `config` called inside `write_object` are ignored: the nested timestamp does
/// not collide with the entry's, and the nested `EntryDimensions` (naming a dimension no field
/// provides) neither leaks `_aws` metadata nor fails the entry with a missing dimension.
#[test]
fn object_timestamp_and_config_are_ignored() {
    struct Sneaky;
    impl ObjectValue for Sneaky {
        fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(2));
            writer.config(
                const {
                    &EntryDimensions::new(Cow::Borrowed(&[Cow::Borrowed(&[Cow::Borrowed(
                        "Missing",
                    )])]))
                },
            );
            writer.value("K", &1u64);
        }
    }
    let doc = render_checked(|| Field("Obj", DirectObject(Sneaky)));
    assert_json_eq!(doc["Obj"], json!({"K": 1}));
    assert_json_eq!(doc["_aws"]["Timestamp"], json!(1000));
    assert_json_eq!(
        doc["_aws"]["CloudWatchMetrics"][0]["Dimensions"],
        json!([[]])
    );
}

/// Two fields sharing a name, written in order.
struct Twice<A, B>(&'static str, A, B);

impl<A: Value + Send + 'static, B: Value + Send + 'static> Entry for Twice<A, B> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        writer.value(self.0, &self.1);
        writer.value(self.0, &self.2);
    }
}

/// An object field's top-level name is duplicate-checked against every earlier field kind.
#[rstest]
#[case::after_string(|| "x")]
#[case::after_metric(|| 5u64)]
#[case::after_object(|| DirectObject(ep()))]
fn duplicate_object_field_name_is_rejected<V: Value + Send + 'static>(
    #[case] prior: impl Fn() -> V,
) {
    let err = render_err_checked(&[], || Twice("Endpoint", prior(), DirectObject(ep())));
    assert!(err.contains("for `Endpoint`: duplicate field"), "{err}");
}

/// An object cannot satisfy a declared dimension: a CloudWatch dimension value must be a string.
#[test]
fn object_in_dimension_field_is_rejected() {
    let err = render_err_checked(&["Operation"], || Field("Operation", DirectObject(ep())));
    assert!(
        err.contains("for `Operation`: can't use object in dimension field"),
        "{err}"
    );
}
