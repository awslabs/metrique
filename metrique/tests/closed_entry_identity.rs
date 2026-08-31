// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Regression coverage for awslabs/metrique#382: with `#[metrics(closeable_entry)]`,
//! a macro-generated closed entry type implements an identity `CloseValue`, so an
//! already-closed entry is a valid closing field of another `#[metrics]` struct
//! without `#[metrics(no_close)]`. The flag is opt-in because adding `CloseValue`
//! for the generated entry is a breaking change.

use assert2::check;
use metrique::{CloseValue, test_util::test_metric, unit_of_work::metrics};

#[metrics(closeable_entry)]
#[derive(Clone)]
struct Inner {
    count: u64,
}

// `InnerEntry` (the generated closed type) is used directly as a closing field,
// with no `#[metrics(no_close)]`. This only compiles because of the identity
// `CloseValue` impl on the generated entry type.
#[metrics]
struct ParentFlatten {
    #[metrics(flatten)]
    inner: InnerEntry,
}

// Compile-time guard: `<Inner as CloseValue>::Closed` names the generated entry
// type, and closing that entry again is the identity (`InnerEntry` closes to
// `InnerEntry`). Both functions fail to compile if either associated type drifts.
fn _closed_type_is_entry(e: InnerEntry) -> <Inner as CloseValue>::Closed {
    e
}
fn _entry_closes_to_itself(e: InnerEntry) -> <InnerEntry as CloseValue>::Closed {
    e
}

// Boundary guard for #382: the identity impl provides `CloseValue` on the entry
// but NOT `CloseValueRef`, so `Arc<InnerEntry>` still requires `#[metrics(no_close)]`
// to be used as a closing field. If that boundary regressed, `Arc<InnerEntry>`
// would gain a `CloseValue` impl and this would need revisiting. See the negative
// reasoning in the resolution notes for awslabs/metrique#382.

#[test]
fn closed_entry_flattens_as_identity() {
    let inner = Inner { count: 7 }.close();
    let entry = test_metric(ParentFlatten { inner });
    check!(entry.metrics["count"] == 7);
}

#[test]
fn closed_entry_close_is_a_no_op() {
    // Closing an already-closed entry is the identity: same type, same rendered
    // value, no matter how many times it is closed.
    let once: InnerEntry = Inner { count: 3 }.close();
    let twice: InnerEntry = once.close();
    let entry = test_metric(ParentFlatten { inner: twice });
    check!(entry.metrics["count"] == 3);
}

#[test]
fn vec_of_closed_entries_closes_as_identity() {
    // The blanket `Vec<V: CloseValue>` impl composes with the identity impl on
    // `InnerEntry`, so `Vec<InnerEntry>` closes to `Vec<InnerEntry>` unchanged.
    // (Rendering a `Vec` of entries as a struct field is a separate writer
    // concern that does not exist on this branch, so we exercise the closing
    // behaviour and render each element individually.)
    let items: Vec<InnerEntry> = vec![Inner { count: 1 }.close(), Inner { count: 2 }.close()];
    let closed: Vec<InnerEntry> = items.close();
    check!(closed.len() == 2);
    for (i, inner) in closed.into_iter().enumerate() {
        let entry = test_metric(ParentFlatten { inner });
        check!(entry.metrics["count"] == (i as u64) + 1);
    }
}
