// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Most applications have some amount of dynamic cross-request state
//! that they would like to attach to their per-request metrics.
//!
//! Native to metrique:
//! - [`metrique::Counter::increment_scoped`]: Increment a shared counter,
//!   but only while a guard is held. Useful for tracking how much
//!   work is in flight.
//!
//! - [`metrique::Witness`]: An atomically swappable value that snapshots
//!   on first access, ensuring flushed metrics match what was seen
//!   during processing.
//!
//! Blanket implementations for `CloseValue`:
//! - [`std::sync::OnceLock`]: Allows const initialization both
//!   for unchanging values and dynamic (with interior mutability).
//!   Closes as `None` if uninitialized.
//!
//! - [`std::sync::Mutex`]: Allows interior mutability with
//!   strongly consistent reads. Handles panicked locks as `None`
//!   on emission.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use metrique::{
    Counter, ServiceMetrics, Witness,
    emf::Emf,
    unit_of_work::metrics,
    writer::{FormatExt, GlobalEntrySink, AttachGlobalEntrySinkExt},
};

// Static / borrowed state: useful when you don't need to inject test values.
static IN_FLIGHT: Counter = Counter::new(0);
static NODE_GROUP: OnceLock<Witness<String>> = OnceLock::new();

fn init_statics() {
    NODE_GROUP.get_or_init(|| {
        let w = Witness::new("unknown".to_string());
        tokio::runtime::Handle::current().spawn(refresh_node_group_forever(w.clone()));
        w
    });
}

// subfield_owned: CloseValueRef can't close `&&Witness/Counter`, only `&Witness/Counter`
#[metrics(subfield_owned)]
struct BorrowedState {
    node_group: &'static OnceLock<Witness<String>>,
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

// Owned state: more flexible, often with Arc + interior mutability.
#[metrics(subfield)]
struct OwnedState {
    active_requests: Counter,
    #[metrics(flatten)]
    app_config: Witness<AppConfig>,
}

#[derive(Debug, Clone, Copy, Default)]
#[metrics(subfield)]
struct AppConfig {
    feature_xyz_enabled: bool,
    throttle_policy: ThrottlePolicy,
}

#[derive(Clone, Copy, Debug, Default)]
#[metrics(value(string))]
enum ThrottlePolicy {
    Throttle,
    #[default]
    NoThrottle,
}

impl OwnedState {
    fn initialize() -> Arc<Self> {
        let state = Arc::new(Self {
            active_requests: Counter::default(),
            app_config: Witness::new(AppConfig::default()),
        });

        let handle = state.clone();
        tokio::task::spawn(refresh_app_config_forever(handle));

        state
    }
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    throttled: bool,
    #[metrics(flatten)]
    static_state: BorrowedState,
    #[metrics(flatten)]
    owned_state: Arc<OwnedState>,
}

impl RequestMetrics {
    fn init(state: &Arc<OwnedState>) -> RequestMetricsGuard {
        RequestMetrics {
            throttled: false,
            static_state: BorrowedState::new(),
            owned_state: state.clone(),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[tokio::main]
async fn main() {
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("Ns".to_string(), vec![vec![]]).output_to_makewriter(std::io::stdout),
    );

    init_statics();
    let state = OwnedState::initialize();

    // Two concurrent requests, both using config from before the first refresh.
    tokio::join!(handle_request(&state), handle_request(&state));

    // Third request, on its own, after the config has refreshed.
    handle_request(&state).await;
}

async fn handle_request(state: &Arc<OwnedState>) {
    let mut metrics = RequestMetrics::init(state);

    // By loading this config, we guarantee the metric will also use the same snapshot.
    let config = metrics.owned_state.app_config.load();
    if matches!(config.throttle_policy, ThrottlePolicy::Throttle) {
        metrics.throttled = true;
    }

    let _guard = IN_FLIGHT.increment_scoped();
    do_some_work().await;
}

async fn do_some_work() {
    tokio::time::sleep(Duration::from_secs(2)).await;
}

async fn refresh_node_group_forever(witness: Witness<String>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        // load from disk, remote, etc
        let _ = witness.store(Arc::new("us-east-1a".to_string()));
    }
}

async fn refresh_app_config_forever(state: Arc<OwnedState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
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

        let _ = state.app_config.store(Arc::new(new_config));
    }
}
