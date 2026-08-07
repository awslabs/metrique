// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Cfg-gated concurrency primitives used by [`super::global`] (std vs. shuttle).
//!
//! Gated on `feature = "_shuttle"` too, not `cfg(shuttle)` alone: `--cfg
//! shuttle` is set process-wide via RUSTFLAGS, so it also reaches builds of
//! this crate (e.g. as a dev-dependency with different requested features)
//! that don't have `_shuttle` enabled and therefore don't have the optional
//! `shuttle` crate linked at all.

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use shuttle::sync::Mutex;
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use std::sync::Mutex;

/// Public (unlike `Mutex` above), since macro-generated code in downstream
/// crates needs to name it directly -- see `global_entry_sink!`'s `ATTACHED`
/// static.
#[doc(hidden)]
#[cfg(all(shuttle, feature = "_shuttle"))]
pub use shuttle::sync::RwLock;
#[doc(hidden)]
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub use std::sync::RwLock;
