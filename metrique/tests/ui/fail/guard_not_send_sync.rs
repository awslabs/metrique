use metrique_writer::sink::global_entry_sink;
use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
global_entry_sink! { TestSink }

fn assert_sync<S: Sync>(_s: &S) {}
fn assert_send<S: Send>(_s: &S) {}

// this is not a test of the macro, but we already have a cfail test
fn main() {
    let TestEntrySink { inspector: _, sink } = test_entry_sink();
    let _guard = TestSink::set_test_sink(sink);
    assert_sync(&_guard);
    assert_send(&_guard);
}

