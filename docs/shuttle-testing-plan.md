# Plan: Shuttle-based deterministic concurrency testing

## Why

`metrique-writer`'s background queue pipeline (`metrique-writer/src/sink/background.rs`)
is a background thread draining a shared queue with hand-documented interleaving
invariants (`WakerTracker`, see the `S1`/`S2`/`L1` comments around line 589). Its
current tests exercise real threads with `std::thread::sleep` polling loops and
60s panic timeouts (e.g. `forget_doesnt_stop_new_entries_from_being_appended`,
`flushes_periodically_even_when_not_writing`, `flush_never_empty`). These are slow,
can flake under CI load, and only sample a handful of the possible thread
interleavings.

[Shuttle](https://github.com/awslabs/shuttle) replaces this class of test with
deterministic, exhaustive-ish scheduling exploration: it drives the same test
body against thousands of interleavings and reports a reproducible seed on
failure. `dial9-tokio-telemetry` (sibling repo, `dial9-core/src/primitives.rs`)
already does this for its own writer -> flush-thread -> sink pipeline and is the
template this plan follows.

The task is broader than just the background queue, though: "background queue
and other primitives" means finding *every* concurrency primitive in this
workspace whose correctness depends on a specific interleaving, not just the
one with the most obviously flaky tests. Auditing the workspace (see Scope,
below) turned up exactly one other genuine candidate — `global.rs`'s
`AttachHandle`/`ShutdownRegistry` `Arc`/`Weak` handshake — and several
`Mutex`/atomic wrappers that look like primitives but aren't, in the sense
that matters for Shuttle (no invariant spans more than one operation).

## Two blockers, not one

### 1. Crossbeam isn't Shuttle-aware

Shuttle only instruments `std` sync primitives it re-implements itself
(`shuttle::sync::{Mutex, RwLock, Condvar, mpsc, atomic}`, `shuttle::thread`).
`background.rs` uses `crossbeam_queue::ArrayQueue` and
`crossbeam_utils::sync::{Parker, Unparker}` directly — Shuttle's scheduler has no
visibility into crossbeam's internal atomics, so a build with `--cfg shuttle`
would not actually explore interleavings inside the queue/parker.

This means the work is not a drop-in feature flag flip. It requires introducing a
`primitives`-style shim (mirroring `dial9-core/src/primitives.rs`) that swaps the
queue and parking implementation for Shuttle-native equivalents under `cfg(shuttle)`,
while leaving the production path (crossbeam) untouched.

### 2. `static`s don't reset between Shuttle iterations

`shuttle::check_pct`/`check_random` re-run the same test closure hundreds or
thousands of times *within one process*. A real Rust `static` (as opposed to a
value freshly constructed inside the test closure) keeps its state across
those iterations — so anything built on genuine global statics can't be
Shuttle-tested by calling through the public global-singleton API; state from
iteration 1 leaks into iteration 2 and invalidates the exploration. This rules
out testing `global_entry_sink!`'s macro-generated `SINK`/`SHUTDOWN_REGISTRY`
statics directly (see Target 2 below) — the fix is to test the underlying
plain struct instead of the static plumbing around it, not to try to shim the
statics themselves. Keep this rule in mind for any future Shuttle target in
this codebase: **if it's a `static`, extract the reusable state into a plain
struct first.**

## Scope

Two concrete targets, chosen because both have genuine interleaving-sensitive
invariants (not just "any code with a `Mutex` in it"):

- **Target 1 — `metrique-writer`'s background queue** (`BackgroundQueue` /
  `Inner` / `Receiver` / `WakerTracker` in
  `metrique-writer/src/sink/background.rs`). See "Key blocker #1" above.
- **Target 2 — `metrique-writer-core`'s global attach/shutdown handshake**
  (`AttachHandle`, `ShutdownRegistry`, the `Arc`/`Weak` dance in `global.rs`
  lines 364-374). `AttachHandle::drop` calls `Arc::try_unwrap(arc)` and treats
  `Err` as `unreachable!()`, reasoning that the macro only ever holds a `Weak`
  reference. That's exactly the kind of "obviously true" invariant that's
  worth having Shuttle try to break: a concurrent `Weak::upgrade` (from
  `shutdown_registry_weak()` racing a drop) could in principle create a second
  strong ref at the wrong moment. See Target 2 section below for how to test
  this without touching the macro's `static`s.

**Explicitly out of scope**, with reasons (don't add these later without a
concrete new invariant to justify them):
- `metrique-core/src/atomics.rs` (`Counter`/`OwnedCounterGuard`) — every
  operation is a single `fetch_add`/`fetch_update` RMW. There's no ordering
  dependency *between* operations for Shuttle to find a bad interleaving of;
  hardware/std already guarantees this. The existing
  `owned_counter_guard_move_across_threads` test is sufficient.
- `metrique-writer/src/sink/mod.rs::VecEntrySink` and
  `sink/immediate_flush.rs::SinkState` — a single `Mutex` guarding a value with
  no wake/park/ordering logic layered on top. `std::sync::Mutex` mutual
  exclusion is not something we need Shuttle to re-verify.
- `global.rs`'s `RuntimeSinkMap`/`ThreadLocalTestSinkGuard` (the `test-util`
  runtime-scoped sink overrides) — same shape as the above (a `Mutex<HashMap>`
  with insert/remove), plus it's test-only code with no invariant beyond "the
  entry doesn't leak past the guard's drop," which existing tests already
  cover directly.

## Changes

### 1. Workspace plumbing

- **`Cargo.toml` (workspace)**: add `shuttle = "0.9"` (match the version pinned
  in `dial9-tokio-telemetry/Cargo.toml`) to `[workspace.dependencies]`, marked
  so it's easy to bump both repos together.
- **`metrique-writer/Cargo.toml`** (Target 1):
  - Add `shuttle = { workspace = true, optional = true }`.
  - Add `unexpected_cfgs = { level = "warn", check-cfg = ["cfg(shuttle)"] }`
    to `[lints.rust]` alongside the existing `metrique_require_explicit_impls`
    entry.
  - Add a `_shuttle` feature: `_shuttle = ["dep:shuttle", "background-queue"]`
    (depends on `background-queue` since that's what pulls in
    crossbeam-queue/crossbeam-utils, the things being shimmed).
- **`metrique-writer-core/Cargo.toml`** (Target 2):
  - Same three additions (`shuttle` optional dep, `check-cfg`, `_shuttle`
    feature), but `_shuttle = ["dep:shuttle"]` with no other feature
    dependency — `global.rs` is unconditionally compiled, not behind a
    feature flag.

Note these are two independent, crate-local `_shuttle` features — not a
shared one. `metrique-writer` depends on `metrique-writer-core`, so in
principle Target 1 could reuse a shim defined in Target 2's crate, but the two
targets shim entirely different things (crossbeam queue/parker vs. plain
`Arc`/`Weak`/`Mutex`), so sharing would just add an indirection with nothing
to reuse. A shared crate (e.g. routing through `metrique-util`) is also the
wrong direction dependency-wise: `metrique-util` currently *depends on*
`metrique-writer-core`, so `metrique-writer-core` cannot depend back on it
without a cycle. Keep each shim local to its own crate.

### 2. Primitives shims

#### Target 1: `metrique-writer/src/sink/shuttle_primitives.rs`

- `cfg(not(shuttle))`: re-export `crossbeam_queue::ArrayQueue` and
  `crossbeam_utils::sync::{Parker, Unparker}` as-is (zero-cost passthrough).
- `cfg(shuttle)`:
  - Replace `ArrayQueue<T>` with a small wrapper around
    `shuttle::sync::Mutex<std::collections::VecDeque<T>>` that implements the
    same `force_push` / `pop` / `len` / `capacity` surface `background.rs`
    actually calls. This mirrors the `BoundedQueue` shim in
    `dial9-core/src/primitives.rs` (lines ~292-327), which exists for exactly
    this reason.
  - Replace `Parker`/`Unparker` with a small wrapper over
    `shuttle::sync::{Mutex<bool>, Condvar}` exposing `park()` /
    `park_deadline()` / `unpark()`, since Shuttle does not model crossbeam's
    `Parker`. (Confirm during implementation whether `shuttle::thread::park`/
    `Thread::unpark` exist and are sufficient instead of hand-rolling
    Condvar — check the installed `shuttle` version's docs before writing the
    Condvar version, to avoid reinventing something already provided.)
  - Also re-export `thread::{spawn, JoinHandle}` and `std::sync::mpsc` vs.
    `shuttle::thread` / `shuttle::sync::mpsc` the same way
    `dial9-core/src/primitives.rs` does, since `background.rs`'s
    `do_build`/`flush_queue_sender` use `std::sync::mpsc` and `thread::Builder`
    directly today.
- Update `background.rs` to import queue/parker/thread/mpsc types from this
  shim module instead of `crossbeam_queue`/`crossbeam_utils`/`std` directly.
  No behavior change under `cfg(not(shuttle))` — purely a rename/re-export.

#### Target 2: `metrique-writer-core/src/global.rs`

Much smaller shim than Target 1's, and — per "Key blocker #2" — deliberately
does **not** touch the macro-generated `static SINK`/`static SHUTDOWN_REGISTRY`
at all:

- `ShutdownRegistry`'s inner `Mutex<Vec<ShutdownFn>>` and `AttachHandle`'s
  `Arc`/`Weak` need `cfg(shuttle)` equivalents
  (`shuttle::sync::{Arc, Mutex, Weak}`), same rename/re-export pattern as
  Target 1, gated behind `metrique-writer-core`'s own `_shuttle` feature.
- The test only needs to construct `ShutdownRegistry`/`AttachHandle` values
  directly (both are already plain, non-static structs — `AttachHandle::new`,
  `shutdown_registry_weak()`, `Drop for AttachHandle` are all callable without
  the `global_entry_sink!` macro or its statics), so no macro-expansion or TLS
  is involved in the shuttle test at all.

### 3. Shuttle test modules

#### Target 1

New `#[cfg(all(test, shuttle))] mod shuttle_tests` in `background.rs` (or a
separate `metrique-writer/src/sink/background_shuttle_tests.rs`, following
`dial9-core`'s convention of splitting large shuttle suites into their own
file, e.g. `pipeline_shuttle_tests.rs`). Port the invariants the sleep-loop
tests are already informally checking, but as direct interleaving checks:

- **Round-trip / no-loss**: entries pushed from N threads are all eventually
  observed by the stream in a valid order (ring-buffer eviction aside), same
  spirit as `writes_all_entries_from_multiple_threads`.
- **`WakerTracker` liveness (L1)**: a flush waker sent before shutdown is
  always eventually woken — replaces the manual "loop + sleep + 60s panic"
  polling used in `flush_simple`/`flush_never_empty`/`flush_without_waiting`.
- **`WakerTracker` safety (S1)**: no waker wakes before its preceding push has
  been observed by the stream.
- **Overflow accounting**: `QueueOverflow` events fire iff `force_push`
  actually evicted an entry, under concurrent push + slow/blocked consumer
  (same scenario as `drops_older_entries_when_full` /
  `custom_observer_records_overflows`, but scheduler-driven instead of
  lock-holding to force the race).
- **Shutdown drains fully**: `shut_down` observes every entry pushed before
  the shutdown signal was set, across interleavings (parallels
  `shut_down_stops_new_entries_from_being_appended`).

Use `shuttle::check_pct(scenario, iterations, thread_count)` for the
probabilistic checks and a bounded `shuttle::check_random`/`check_dfs` (whichever
`dial9-core` prefers — confirm by re-reading `pipeline_shuttle_tests.rs`
against the installed shuttle version) for anything needing full determinism
around a fixed number of threads. Existing sleep-loop tests stay as-is (real
end-to-end regression coverage); the shuttle tests are additive, not a
replacement.

#### Target 2

New `#[cfg(all(test, shuttle))] mod shuttle_tests` in `global.rs`:

- **No lost/duplicated shutdown invariant**: build an `AttachHandle`, take
  `shutdown_registry_weak()` on one (simulated) thread, drop the `AttachHandle`
  on another, and assert exactly one of "drain_and_run ran" / "weak upgrade
  observed a live registry" happens — never both, never neither. This is
  directly probing whether `Arc::try_unwrap`'s `unreachable!()` branch at
  `global.rs:370` really is unreachable, rather than trusting the comment.
- **LIFO shutdown order under concurrent `push`**: multiple threads calling
  `ShutdownRegistry::push` concurrently before the handle drops, then
  asserting `drain_and_run` still runs them in a valid LIFO-per-thread order.

### 4. Local + CI scripts

- **`scripts/test-shuttle.sh`** (new, matching
  `dial9-tokio-telemetry/scripts/test-shuttle.sh`, but covering both targets —
  same two-crate-invocation shape as dial9's own script, which runs
  `-p dial9-core` then `-p dial9-tokio-telemetry`):
  ```bash
  #!/usr/bin/env bash
  set -euo pipefail
  RUSTFLAGS="--cfg shuttle" \
    cargo test -p metrique-writer-core --lib --features _shuttle -- shuttle "$@"
  RUSTFLAGS="--cfg shuttle" \
    cargo test -p metrique-writer --lib --features _shuttle -- shuttle "$@"
  ```
- **`scripts/ci-local.sh`**: add a call to `scripts/test-shuttle.sh` in the
  same place the existing nextest matrix runs, so `ci-local.sh` stays the
  single source of truth for "what CI runs."
- **CI workflow**: add a `shuttle` job to `.github/workflows/ci.yml` (or a
  dedicated `shuttle.yml` if the existing `ci.yml` is already large/matrixed —
  check its structure before deciding) that runs `scripts/test-shuttle.sh`.
  Consider whether it belongs in the regular `ci.yml` (fast feedback per-PR)
  or in `stress-test.yml`'s nightly/scheduled job (Shuttle runs can be slower
  than a normal unit test pass) — recommend starting in `ci.yml` with a modest
  iteration count, and adding a nightly high-iteration run via
  `scripts/shuttle-coverage.sh` (see below) only if phase-1 coverage proves
  valuable.
- **`scripts/shuttle-coverage.sh`** (new, matching
  `dial9-tokio-telemetry/scripts/shuttle-coverage.sh`): `cargo llvm-cov --html`
  wrapper for local coverage inspection, not wired into CI. Extend to both
  `-p metrique-writer-core` and `-p metrique-writer`.

### 5. Docs

- Add a short "Shuttle" section to `AGENTS.md` (this repo's own agent
  guidelines file) describing the `_shuttle` feature, `test-shuttle.sh`, and
  the rule that it's a separate invocation from `cargo nextest run` — mirroring
  the equivalent section in `dial9-tokio-telemetry/AGENTS.md`, so future agents
  in this repo don't miss it the same way they wouldn't in dial9.

## Verification (as actually run)

- `cargo test --workspace --all-features --lib` (nextest isn't installed in
  this environment; `cargo test` is the fallback the AGENTS.md testing section
  already names) passes unchanged across the whole workspace — both shims are
  a no-op under `cfg(not(shuttle))`.
- `./scripts/test-shuttle.sh` passes: Target 1's 5 tests pass for real: a
  deliberately-introduced bug in `Parker`/`Unparker` (an earlier, thread-
  identity-bound version of the shim) made them deadlock deterministically
  until fixed, confirming they exercise genuine interleavings rather than
  passing vacuously. Target 2's 2 tests are `#[ignore]`d: they reproducibly
  find a real bug in `AttachHandle::drop` (see `docs/shuttle-findings.md`),
  left unfixed and documented rather than patched as a side effect of adding
  test coverage.
- `RUSTFLAGS="--cfg tokio_unstable" cargo clippy --workspace --all-features -- -D warnings`
  and `cargo +nightly fmt --all -- --check` both clean.
- No change in behavior of the non-shuttle build — the two pre-existing
  doctest failures seen in `cargo test --features test-util` (an unrelated
  `metrics_024`/`MetricsRsVersion` version mismatch, and a `service-metrics`
  feature-gating issue in a `metrique-writer-core` doctest) were confirmed via
  `git stash` to already exist on unmodified `main`, unrelated to this work.

## Open questions — resolved during implementation

1. **Does shuttle 0.9.1 expose `thread::park`/`Thread::unpark`?** Yes, but
   *don't* build the `Parker`/`Unparker` shim on them: they're bound to the
   calling thread's identity, and `background.rs` constructs the `Parker` on
   one thread (`do_build`, the caller) while the *spawned* background thread
   is the one that later calls `park()`. An identity-bound shim captures the
   wrong thread and deadlocks — confirmed empirically (see below). The shim
   uses a `Mutex<bool>` + `Condvar` token shared via `Arc` instead, matching
   crossbeam's actual (thread-identity-agnostic) semantics.
2. **`shuttle::sync::Weak`?** Turned out to be moot: shuttle 0.9.1 doesn't
   instrument `Arc`/`Weak` at all (`sync/mod.rs`: `// TODO implement true
   support for Arc` — it's a bare `pub use std::sync::{Arc, Weak}`). Target
   2's shim therefore only swaps `ShutdownRegistry`'s internal `Mutex`; `Arc`/
   `Weak` stay as std's throughout, unconditionally.
3. Not resolved — out of scope once CI wiring turned out to be a smaller
   addition than expected (see Changes §4 as implemented).
4. **Resolved: yes, a real bug.** The `concurrent_register_and_drop` shuttle
   test (added under `metrique-writer-core/src/global.rs`) reproducibly finds
   a live bug in `AttachHandle::drop`'s `unreachable!()` branch — a concurrent
   `register_shutdown_fn` can hold a genuine second strong `Arc` at the exact
   moment `drop` calls `try_unwrap`, which panics instead of handling it. Per
   this question's own guidance, this was **not** fixed as part of adding test
   coverage — the test is `#[ignore]`d with a pointer to
   `docs/shuttle-findings.md`, which documents the bug, the repro, and the
   open design question (what *should* happen when registration races
   shutdown?) that a real fix needs to answer.

### Unplanned finding: "atomic-looking" code needs an explicit yield to be explorable

The first version of the Target 2 test (no explicit yield between spawning
the racing thread and calling `drop`) passed 50,000 iterations without ever
exercising the actual race — not because the race doesn't exist, but because
`AttachHandle::drop`'s body touches no shuttle-instrumented primitive, so it
looks atomic to the scheduler: there's no yield point inside it for the
scheduler to use to interleave another thread *during* it. Without an
explicit `shuttle::thread::yield_now()` on the racing side, shuttle could only
ever schedule the two threads fully-before-or-fully-after each other. This is
a general lesson for any future shuttle test in this codebase: a scenario
built entirely from operations that don't individually yield (spawn, an
uninstrumented function body, join) may need an explicit yield point inserted
to actually be explorable — passing tests silently under-covering the thing
they're meant to test is a worse failure mode than a slow test.
