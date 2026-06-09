//! Extracts dimension `(class, instance)` pairs from an entry by traversing it with a no-op writer.

use metrique_writer::{EntryConfig, EntryWriter, Value, ValueWriter};
use metrique_writer_core::value::{MetricFlags, Observation};
use metrique_writer_core::{Unit, ValidationError};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::time::SystemTime;

type CowStr = Cow<'static, str>;

/// Extract all `(class, instance)` dimension pairs from an entry, canonicalized (sorted + dedup'd).
pub fn extract_dimensions(
    entry: &(impl metrique_core::InflectableEntry + ?Sized),
) -> SmallVec<[(CowStr, CowStr); 4]> {
    let mut probe = DimensionProbeWriter {
        dimensions: SmallVec::new(),
    };
    entry.write(&mut probe);
    probe.dimensions.sort();
    probe.dimensions.dedup();
    probe.dimensions
}

struct DimensionProbeWriter {
    dimensions: SmallVec<[(CowStr, CowStr); 4]>,
}

impl<'a> EntryWriter<'a> for DimensionProbeWriter {
    fn timestamp(&mut self, _timestamp: SystemTime) {}

    fn value(&mut self, _name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        value.write(DimensionProbeValueWriter {
            dimensions: &mut self.dimensions,
        });
    }

    fn config(&mut self, _config: &'a dyn EntryConfig) {}
}

struct DimensionProbeValueWriter<'a> {
    dimensions: &'a mut SmallVec<[(CowStr, CowStr); 4]>,
}

impl ValueWriter for DimensionProbeValueWriter<'_> {
    fn string(self, _value: &str) {}

    fn metric<'a>(
        self,
        _distribution: impl IntoIterator<Item = Observation>,
        _unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        for (class, instance) in dimensions {
            self.dimensions.push((
                Cow::Owned(class.to_owned()),
                Cow::Owned(instance.to_owned()),
            ));
        }
    }

    fn error(self, _error: ValidationError) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrique_writer::MetricValue;
    use metrique_writer::value::{WithDimension, WithDimensions};
    use std::time::Duration;

    struct SingleDimEntry {
        value: WithDimension<u64>,
    }
    impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for SingleDimEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("value", &self.value);
        }
    }

    #[test]
    fn empty_entry() {
        struct Empty;
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Empty {
            fn write<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {}
        }
        let dims = extract_dimensions(&Empty);
        assert!(dims.is_empty());
    }

    #[test]
    fn single_dimension() {
        let entry = SingleDimEntry {
            value: 42u64.with_dimension("Event", "GET"),
        };
        let dims = extract_dimensions(&entry);
        assert_eq!(
            dims.as_slice(),
            &[(Cow::Borrowed("Event"), Cow::Borrowed("GET"))]
        );
    }

    #[test]
    fn multiple_dimensions_different_classes() {
        struct Multi {
            a: WithDimension<u64>,
            b: WithDimension<u64>,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Multi {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("a", &self.a);
                writer.value("b", &self.b);
            }
        }
        let entry = Multi {
            a: 1u64.with_dimension("Event", "GET"),
            b: 2u64.with_dimension("Region", "us-east-1"),
        };
        let dims = extract_dimensions(&entry);
        assert_eq!(dims.len(), 2);
        assert!(dims.contains(&(Cow::Borrowed("Event"), Cow::Borrowed("GET"))));
        assert!(dims.contains(&(Cow::Borrowed("Region"), Cow::Borrowed("us-east-1"))));
    }

    #[test]
    fn dedup_identical_pairs() {
        struct Dup {
            a: WithDimension<u64>,
            b: WithDimension<u64>,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Dup {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("a", &self.a);
                writer.value("b", &self.b);
            }
        }
        let entry = Dup {
            a: 1u64.with_dimension("Event", "GET"),
            b: 2u64.with_dimension("Event", "GET"),
        };
        let dims = extract_dimensions(&entry);
        assert_eq!(dims.len(), 1);
    }

    #[test]
    fn same_class_different_instance() {
        struct Multi {
            a: WithDimension<u64>,
            b: WithDimension<u64>,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Multi {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("a", &self.a);
                writer.value("b", &self.b);
            }
        }
        let entry = Multi {
            a: 1u64.with_dimension("Event", "GET"),
            b: 2u64.with_dimension("Event", "POST"),
        };
        let dims = extract_dimensions(&entry);
        assert_eq!(dims.len(), 2);
    }

    #[test]
    fn sorting_canonicalization() {
        struct Multi {
            a: WithDimension<u64>,
            b: WithDimension<u64>,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Multi {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("a", &self.a);
                writer.value("b", &self.b);
            }
        }
        let entry = Multi {
            a: 1u64.with_dimension("Zebra", "z"),
            b: 2u64.with_dimension("Alpha", "a"),
        };
        let dims = extract_dimensions(&entry);
        assert_eq!(dims[0], (Cow::Borrowed("Alpha"), Cow::Borrowed("a")));
        assert_eq!(dims[1], (Cow::Borrowed("Zebra"), Cow::Borrowed("z")));
    }

    #[test]
    fn runtime_set_dimensions_via_add_dimension() {
        let mut val: WithDimensions<u64, 2> = WithDimensions::from(10u64);
        val.add_dimension("Year", "2025");
        val.add_dimension("Season", "Spring");

        struct DynEntry {
            value: WithDimensions<u64, 2>,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for DynEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("value", &self.value);
            }
        }
        let entry = DynEntry { value: val };
        let dims = extract_dimensions(&entry);
        assert_eq!(dims.len(), 2);
        assert!(dims.contains(&(Cow::Borrowed("Season"), Cow::Borrowed("Spring"))));
        assert!(dims.contains(&(Cow::Borrowed("Year"), Cow::Borrowed("2025"))));
    }

    #[test]
    fn nested_via_flatten() {
        struct Inner {
            value: WithDimension<Duration>,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Inner {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("value", &self.value);
            }
        }
        struct Outer {
            inner: Inner,
        }
        impl<NS: metrique_core::NameStyle> metrique_core::InflectableEntry<NS> for Outer {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                metrique_core::InflectableEntry::<NS>::write(&self.inner, writer);
            }
        }
        let entry = Outer {
            inner: Inner {
                value: Duration::from_millis(10).with_dimension("Op", "Read"),
            },
        };
        let dims = extract_dimensions(&entry);
        assert_eq!(dims.len(), 1);
        assert_eq!(dims[0], (Cow::Borrowed("Op"), Cow::Borrowed("Read")));
    }
}
