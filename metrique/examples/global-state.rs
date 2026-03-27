// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared state in per-request metrics.
//!
//! Most applications have dynamic cross-request state (feature flags,
//! routing config, in-flight counters) that should appear on every
//! metric record for correlation during debugging.
//!
//! This example shows two approaches:
//!
//! 1. **Borrowed (`&'static`)**: `Counter` and `OnceLock<State<T>>`
//!    live in statics. Cheap, no `Arc` overhead, but not injectable
//!    for tests. The `OnceLock` here is purely for lazy initialization
//!    of the `State`; the snapshot behavior comes from `State` itself.
//!
//! 2. **Owned (clone-per-request)**: state lives in an `AppState` struct
//!    that is cloned into each request's metrics. Cloning shares the
//!    underlying `Counter` (via `Arc`) and gives each request a fresh
//!    `State` snapshot slot. More flexible, easy to inject in tests.
//!
//! Both patterns flatten shared state into the per-request metric, so
//! every emitted record includes the current counter value, config
//! snapshot, etc.
//!
//! Key primitives used:
//! - [`State<T>`](metrique_util::State): atomically swappable value with
//!   snapshot-on-first-read semantics.
//! - [`Counter::increment_scoped`](metrique::Counter::increment_scoped):
//!   in-flight tracking with automatic decrement on drop.
//! - [`OnceLock<T>`](std::sync::OnceLock): lazy one-time initialization
//!   with `CloseValue` support (closes as `None` if uninitialized).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use metrique::{
    Counter, ServiceMetrics,
    emf::Emf,
    unit_of_work::metrics,
    writer::{AttachGlobalEntrySinkExt, FormatExt, GlobalEntrySink},
};
use metrique_util::State;

// ---------------------------------------------------------------------------
// Borrowed (static) state
// ---------------------------------------------------------------------------

static IN_FLIGHT: Counter = Counter::new(0);
static NODE_GROUP: OnceLock<State<String>> = OnceLock::new();

fn init_statics() {
    NODE_GROUP.get_or_init(|| {
        let w = State::new("unknown".to_string());
        tokio::runtime::Handle::current().spawn(refresh_node_group_forever(w.clone()));
        w
    });
}

// subfield_owned: CloseValueRef can't close `&&State/Counter`, only `&State/Counter`
#[metrics(subfield_owned)]
struct BorrowedState {
    node_group: &'static OnceLock<State<String>>,
    in_flight: &'static Counter,
}

impl BorrowedState {
    fn new() -> Self {
        Self {
            node_group: &NODE_GROUP,
            in_flight: &IN_FLIGHT,
        }
    }
}

// ---------------------------------------------------------------------------
// Owned (clone-per-request) state
// ---------------------------------------------------------------------------

// Cloning shares the Counter (via Arc) and gives a fresh State snapshot slot.
#[derive(Clone)]
#[metrics(subfield_owned)]
struct AppState {
    active_requests: Arc<Counter>,
    #[metrics(flatten)]
    app_config: State<AppConfig>,
}

#[derive(Debug, Clone, Copy, Default)]
#[metrics(subfield)]
struct AppConfig {
    feature_xyz_enabled: bool,
    throttle_policy: ThrottlePolicy,
}

#[derive(Default)]
#[metrics(value(string))]
enum ThrottlePolicy {
    Throttle,
    #[default]
    NoThrottle,
}

impl AppState {
    fn initialize() -> Self {
        let state = Self {
            active_requests: Arc::new(Counter::default()),
            app_config: State::new(AppConfig::default()),
        };

        tokio::task::spawn(refresh_app_config_forever(state.clone()));

        state
    }
}

// ---------------------------------------------------------------------------
// Per-request metrics
// ---------------------------------------------------------------------------

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    throttled: bool,
    #[metrics(flatten)]
    static_state: BorrowedState,
    #[metrics(flatten)]
    app_state: AppState,
}

impl RequestMetrics {
    fn init(state: &AppState) -> RequestMetricsGuard {
        RequestMetrics {
            throttled: false,
            static_state: BorrowedState::new(),
            app_state: state.clone(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to_makewriter(std::io::stdout),
    );

    init_statics();
    let state = AppState::initialize();

    // Two concurrent requests, both using config from before the first refresh.
    tokio::join!(handle_request(&state), handle_request(&state));

    // Third request, on its own, after the config has refreshed.
    handle_request(&state).await;

    // Example output (timestamps will vary):
    //
    // Requests 1 and 2 see the default config (NoThrottle, feature off).
    // Request 3 starts after the 1s refresh, sees the new config (Throttle, feature on).
    //
    // {"Throttled":0,"InFlight":1,"ActiveRequests":0,"FeatureXyzEnabled":0,"NodeGroup":"us-east-1a","ThrottlePolicy":"NoThrottle", ...}
    // {"Throttled":0,"InFlight":0,"ActiveRequests":0,"FeatureXyzEnabled":0,"NodeGroup":"us-east-1a","ThrottlePolicy":"NoThrottle", ...}
    // {"Throttled":1,"InFlight":0,"ActiveRequests":0,"FeatureXyzEnabled":1,"NodeGroup":"us-east-1a","ThrottlePolicy":"Throttle", ...}
}

async fn handle_request(state: &AppState) {
    let mut metrics = RequestMetrics::init(state);

    // Loading here to branch on the config; this also pins the metric
    // snapshot to this point rather than emission time.
    let config = metrics.app_state.app_config.snapshot();
    if matches!(config.throttle_policy, ThrottlePolicy::Throttle) {
        metrics.throttled = true;
    }

    let _guard = IN_FLIGHT.increment_scoped();
    do_some_work().await;
}

async fn do_some_work() {
    tokio::time::sleep(Duration::from_millis(1500)).await;
}

// ---------------------------------------------------------------------------
// Background refresh tasks
// ---------------------------------------------------------------------------

async fn refresh_node_group_forever(state: State<String>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        // load from disk, remote, etc
        state.store(Arc::new("us-east-1a".to_string()));
    }
}

async fn refresh_app_config_forever(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.tick().await; // skip the immediate first tick
    let mut i = 0;
    loop {
        interval.tick().await;
        i += 1;

        let new_config = if i % 2 == 0 {
            AppConfig {
                feature_xyz_enabled: false,
                throttle_policy: ThrottlePolicy::NoThrottle,
            }
        } else {
            AppConfig {
                feature_xyz_enabled: true,
                throttle_policy: ThrottlePolicy::Throttle,
            }
        };

        state.app_config.store(Arc::new(new_config));
    }
}
