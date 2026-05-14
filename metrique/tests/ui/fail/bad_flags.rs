use metrique::unit_of_work::metrics;

struct FakeFlagCtor;
impl metrique::writer::value::FlagConstructor for FakeFlagCtor {
    fn construct() -> metrique::writer::MetricFlags<'static> {
        todo!()
    }
}

#[metrics]
struct FlagsOnFlatten {
    #[metrics(flatten, flags(FakeFlagCtor))]
    inner: Inner,
}

#[metrics(subfield)]
struct Inner {
    x: u64,
}

#[metrics]
struct FlagsOnFlattenEntry {
    #[metrics(flatten_entry, flags(FakeFlagCtor))]
    inner: FlatEntry,
}

#[derive(metrique::writer::Entry)]
struct FlatEntry {
    x: u64,
}

#[metrics]
struct FlagsOnIgnore {
    #[metrics(ignore, flags(FakeFlagCtor))]
    ignored: u64,
}

#[metrics]
struct FlagsOnTimestamp {
    #[metrics(timestamp, flags(FakeFlagCtor))]
    ts: std::time::SystemTime,
}

fn main() {}
