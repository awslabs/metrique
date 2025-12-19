use metrique_timesource::{TimeSource, set_time_source};

use std::time::UNIX_EPOCH;

fn assert_sync<S: Sync>(_s: &S) {}
fn assert_send<S: Send>(_s: &S) {}

// this is not a test of the macro, but we already have a cfail test
fn main() {   
    let _mock_time = set_time_source(TimeSource::tokio(UNIX_EPOCH));
    assert_sync(&_mock_time);
    assert_send(&_mock_time);
}
