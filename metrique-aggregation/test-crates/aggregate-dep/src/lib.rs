// SPDX-License-Identifier: Apache-2.0

//! Test fixture: a library crate that exposes an entry-mode `#[aggregate]` struct.
//! Used to verify that cross-crate coherence (E0119) does not regress.

use metrique::timers::Timer;
use metrique::unit::Millisecond;
use metrique::unit_of_work::metrics;
use metrique_aggregation::{aggregate, histogram::Histogram};
use std::time::Duration;

/// An entry-mode aggregate defined in a *dependency* crate.
/// A second entry-mode aggregate in the downstream crate must not cause E0119.
#[aggregate]
#[metrics]
pub struct DepMetrics {
    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    pub latency: Timer,
}
