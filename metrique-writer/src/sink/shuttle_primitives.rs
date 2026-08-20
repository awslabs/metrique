// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Cfg-gated concurrency primitives used by [`super::background`] (crossbeam/std
//! vs. shuttle).
//!
//! Normally this re-exports the production crossbeam types. With `--cfg shuttle`
//! it substitutes shuttle-native equivalents: shuttle has no visibility into
//! crossbeam's (or plain `std::sync::atomic`'s) internal atomics -- also why
//! `Arc` itself isn't swapped: nothing depends on *when* its refcount hits
//! zero here. The shuttle-side substitutes (`ArrayQueue`, `Parker`/`Unparker`,
//! `deadline_reached`) live in `metrique_writer_core::shuttle_test_support`,
//! shared with sibling crates that need the same shims.
//!
//! Gated on `feature = "_shuttle"` too, not `cfg(shuttle)` alone: `--cfg shuttle`
//! is set process-wide via RUSTFLAGS, so it also reaches builds of this crate
//! (e.g. as a dev-dependency with different requested features) that don't have
//! `_shuttle` enabled and therefore don't have the optional `shuttle` crate
//! linked at all.

#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use std::{
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc,
    thread,
};

#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use crossbeam_queue::ArrayQueue;
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use crossbeam_utils::sync::{Parker, Unparker};

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use shuttle::{
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc,
    thread,
};

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use metrique_writer_core::shuttle_test_support::{
    ArrayQueue, Parker, Unparker, deadline_reached,
};

/// Whether `now` has reached `deadline`. Trivial outside shuttle.
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) fn deadline_reached(now: std::time::Instant, deadline: std::time::Instant) -> bool {
    now >= deadline
}
