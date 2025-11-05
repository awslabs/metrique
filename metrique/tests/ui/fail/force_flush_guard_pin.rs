use metrique::ServiceMetrics;
use metrique::unit_of_work::metrics;
use metrique::writer::GlobalEntrySink;

#[metrics]
struct Foo {
}

fn assert_unpin<T: Unpin>(_t: T) {}

fn main() {
    let m = Foo {}.append_on_drop(ServiceMetrics::sink());
    assert_unpin(m.force_flush_guard());
}
