# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.26](https://github.com/awslabs/metrique/compare/metrique-v0.1.25...metrique-v0.1.26) - 2026-06-09

### Added

- **`metrique-otel`**: New crate for OpenTelemetry integration. Records metrique entries directly onto an OTel `SdkMeterProvider` with OTLP/gRPC export. Use `flags(...)` to select OTel instrument kinds; non-OTel sinks ignore the flags, so the same struct works in mixed topologies. ([#291](https://github.com/awslabs/metrique/pull/291))

  ```rust
  use metrique_otel::OtelSink;
  use metrique_otel::flags::Counter;

  #[aggregate]
  #[metrics(rename_all = "PascalCase")]
  struct RequestMetrics {
      #[aggregate(key)]
      operation: String,
      #[aggregate(strategy = Sum)]
      #[metrics(flags(Counter))]
      request_count: u64,
      #[aggregate(strategy = Histogram<Duration>)]
      #[metrics(unit = Millisecond)]
      latency: Duration,
  }
  ```

- *(metrique-util)* System metrics bridge via `sysinfo` (feature: `sysinfo-bridge`). Periodically samples CPU, memory, swap, load average, uptime, and current-process metrics. Opt-in disk, network, and thermal metrics via builder methods. ([#283](https://github.com/awslabs/metrique/pull/283))

  ```rust
  use metrique_util::{AttachGlobalEntrySinkSysinfoExt, SysinfoMetricsConfig};

  ServiceMetrics::subscribe_sysinfo_metrics(
      SysinfoMetricsConfig::default()
          .with_interval(Duration::from_secs(30))
          .with_name_style(MetricNameStyle::KebabCase)
          .with_disks()
          .with_networks(),
  );
  ```

- *(macro)* `#[metrics(flags(...))]` field attribute for per-field `ForceFlag` without wrapping the type. Multiple flags can be combined on a single field. ([#290](https://github.com/awslabs/metrique/pull/290))

  ```rust
  use metrique::emf::flags::{HighStorageResolution, NoMetric};

  #[metrics(rename_all = "PascalCase")]
  struct Metrics {
      #[metrics(flags(HighStorageResolution))]
      latency: u64,
      #[metrics(flags(NoMetric))]
      debug_count: u64,
  }
  ```

- Observer hooks for `BackgroundQueue` and `FlushImmediately` sinks. Backend-agnostic trait replaces the deprecated internal `MetricRecorder` approach—any closure of the right shape is an observer. ([#296](https://github.com/awslabs/metrique/pull/296))

  ```rust
  use metrique_writer::sink::{BackgroundQueueBuilder, BackgroundQueueEvent};

  let queue = BackgroundQueueBuilder::new().observer(move |_queue: &str, event| {
      if let BackgroundQueueEvent::QueueOverflow { .. } = event {
          overflow_counter.fetch_add(1, Ordering::Relaxed);
      }
  });
  ```

- **`metrique-writer-cloudwatch`**: New crate providing a CloudWatch Logs `PutLogEvents` (EMF) `EntryIoStream` backend. Channel-based async architecture with bounded backpressure, respects CW Logs limits (1MB payload, 10k events), graceful shutdown, and auto-creates log group/stream on startup. ([#302](https://github.com/awslabs/metrique/pull/302))

### Changed

- *(metrique-writer-cloudwatch)* **Breaking:** Gate default Tokio runtime behind `tokio_runtime` feature. `TaskSpawner::tokio()` now requires the feature; the spawner no longer returns a `JoinHandle` but spawns-and-detaches instead. ([#305](https://github.com/awslabs/metrique/pull/305))

### Fixed

- *(metrique-writer-cloudwatch)* Avoid legacy AWS client stack that was pulling in old rustls/hyper dependencies causing critical vulnerability findings ([#306](https://github.com/awslabs/metrique/pull/306))
- Deterministically sort keys in snapshot test ([#303](https://github.com/awslabs/metrique/pull/303))

### Other

- Add [`_guide::extending`](https://docs.rs/metrique/latest/metrique/_guide/extending/) documentation covering the closing model, core traits, and copy-paste recipes for custom accumulators, value wrappers, and manual `Entry` impls ([#297](https://github.com/awslabs/metrique/pull/297))
- Clean up `GlobalEntrySink` description ([#292](https://github.com/awslabs/metrique/pull/292))
- Fix race condition in worker tests ([#285](https://github.com/awslabs/metrique/pull/285))
- Make worker sink exit cleanly ([#284](https://github.com/awslabs/metrique/pull/284))
- *(metrique-util)* Mention pending-sink in README ([#279](https://github.com/awslabs/metrique/pull/279))

## [0.1.25](https://github.com/awslabs/metrique/compare/metrique-v0.1.24...metrique-v0.1.25) - 2026-04-20

### Added

- `pending_sink` in `metrique-util` (feature: `pending-sink`): deferred sink attachment with bounded buffering. ([#275](https://github.com/awslabs/metrique/pull/275))
  - Emit metrics immediately during startup while the real sink initializes asynchronously.
  - Entries buffer in a lock-free ring buffer; oldest dropped when full.
  - `resolver.resolve(real_sink)` drains the buffer and switches to direct forwarding.

  ```rust
  use metrique_util::pending_sink;

  // Create a pending sink that buffers up to 1024 entries.
  let (sink, resolver) = pending_sink::new(1024);

  // Attach immediately so metrics start buffering during startup.
  let _handle = ServiceMetrics::attach((sink, ()));

  // Later, once the real sink is ready (after async init, credential
  // exchange, etc.):
  resolver.resolve(real_sink);
  // Buffered entries drain into the real sink; future appends go direct.
  ```

### Other

- Fix audit ([#276](https://github.com/awslabs/metrique/pull/276))
- reframe terminology to lead with "wide events" ([#273](https://github.com/awslabs/metrique/pull/273))
- use workspace dependencies for inter-crate deps ([#271](https://github.com/awslabs/metrique/pull/271))

## [0.1.24](https://github.com/awslabs/metrique/compare/metrique-v0.1.23...metrique-v0.1.24) - 2026-04-13

### Added

- Tokio runtime metrics integration via `metrique-util` (feature: `tokio-metrics-bridge`). Spawns a background reporter that periodically appends `RuntimeMetrics` snapshots (worker utilization, park counts, queue depths, poll durations, etc.) to the attached sink. The reporter is automatically aborted when the attach handle drops. ([#256](https://github.com/awslabs/metrique/pull/256))

  ```rust
  use metrique_util::{AttachGlobalEntrySinkTokioMetricsExt, TokioRuntimeMetricsConfig};

  let _handle = ServiceMetrics::attach_to_stream(
      Emf::all_validations("MyApp".to_string(), vec![vec![]])
          .output_to(std::io::stderr()),
  );

  let config = TokioRuntimeMetricsConfig::default()
      .with_interval(Duration::from_secs(30))
      .with_name_style(MetricNameStyle::KebabCase);
  ServiceMetrics::subscribe_tokio_runtime_metrics(config);
  ```

- `Vec<V: Value>` and `[V: Value]` now implement `Value`, so vector fields emit as native JSON arrays in EMF and comma-joined strings in other formats. Elements that write nothing (e.g. `None` in `Vec<Option<String>>`) are skipped automatically. ([#266](https://github.com/awslabs/metrique/pull/266))

  ```rust
  #[metrics(rename_all = "PascalCase")]
  struct RequestMetrics {
      plugins: Vec<String>,
      request_count: u32,
  }
  // EMF output: {"Plugins": ["auth", "cache"], "RequestCount": 5}
  // Default:   Plugins=auth,cache
  ```

- `OwnedCounterGuard`: an owned variant of `CounterGuard` that holds an `Arc<Counter>` instead of a reference, allowing it to be moved across async boundaries or stored in structs without lifetime constraints. Returned by `Counter::increment_owned`. ([#265](https://github.com/awslabs/metrique/pull/265))

  ```rust
  use std::sync::Arc;
  use metrique::Counter;

  let counter = Arc::new(Counter::new(0));
  let (guard, count) = counter.increment_owned(); // +1 now, -1 on drop
  // guard can be stored in a response body wrapper, moved into a spawned task, etc.
  tokio::spawn(async move {
      do_work().await;
      drop(guard); // decrements when the task completes
  });
  ```

### Fixed

- Convert broken reference-style links to inline links in README ([#262](https://github.com/awslabs/metrique/pull/262))

## [0.1.23](https://github.com/awslabs/metrique/compare/metrique-v0.1.22...metrique-v0.1.23) - 2026-04-01

### Added

- Graceful no-op when no sink is attached ([#251](https://github.com/awslabs/metrique/pull/251))

### Fixed

- *(metrics)* Make metrics macro inside macro_rules more hygiene-safe and cfg-aware ([#259](https://github.com/awslabs/metrique/pull/259))
- *(metrics)* Derive debug through metrics macro ([#257](https://github.com/awslabs/metrique/pull/257))
- *(doc)* Make `*.md` links work on crates.io/github also ([#254](https://github.com/awslabs/metrique/pull/254))

### Other

- Add RenderQueue sink ([#253](https://github.com/awslabs/metrique/pull/253))

## [0.1.22](https://github.com/awslabs/metrique/compare/metrique-v0.1.21...metrique-v0.1.22) - 2026-03-17

### Fixed

- *(doc)* Fix docs.rs build failure for `metrique` crate ([#245](https://github.com/awslabs/metrique/pull/245))

## [0.1.21](https://github.com/awslabs/metrique/compare/metrique-v0.1.20...metrique-v0.1.21) - 2026-03-17

### Added

- `metrique-util` crate with `State<T>` (feature: `state`): atomically swappable shared value with snapshot-on-first-read semantics ([#235](https://github.com/awslabs/metrique/pull/235))

  ```rust
  let shared = State::new(AppConfig::default());
  shared.store(Arc::new(new_config)); // background task swaps in new config
  let request = shared.clone();       // each request clones a handle
  let config = request.snapshot();    // first snapshot() pins the value
  ```

- `Counter::increment_scoped`: returns a guard that decrements on drop, for tracking in-flight work ([#235](https://github.com/awslabs/metrique/pull/235))

  ```rust
  static IN_FLIGHT: Counter = Counter::new(0);
  let _guard = IN_FLIGHT.increment_scoped(); // +1 now, -1 on drop
  ```

- `Counter::new` is now `const fn`, enabling `static Counter` declarations ([#235](https://github.com/awslabs/metrique/pull/235))
- `CloseValue` impl for `CounterGuard` and `OnceLock<T>` ([#235](https://github.com/awslabs/metrique/pull/235))
- Pure JSON output format via `metrique::json::Json` ([#224](https://github.com/awslabs/metrique/pull/224))

  ```rust
  let _handle = ServiceMetrics::attach_to_stream(
      Json::new().output_to_makewriter(|| std::io::stdout().lock()),
  );
  // {"timestamp":...,"metrics":{...},"properties":{...}}
  ```

- `skip_validate_dimensions_exist` setter on `EmfBuilder` ([#223](https://github.com/awslabs/metrique/pull/223))

### Fixed

- EMF produced trailing commas when distribution ends with skipped observation ([#222](https://github.com/awslabs/metrique/pull/222))

### Other

- Add [`_guide`](https://docs.rs/metrique/latest/metrique/_guide/) module with cookbook, concurrency, sinks, sampling, testing ([#219](https://github.com/awslabs/metrique/pull/219), [#232](https://github.com/awslabs/metrique/pull/232))
- *(examples)* add global-state example combining `State`, `Counter::increment_scoped`, and `OnceLock` ([#235](https://github.com/awslabs/metrique/pull/235))
- *(examples)* use metrique::ServiceMetrics instead of global_entry_sink! ([#241](https://github.com/awslabs/metrique/pull/241))
- Add metrique json feature and formatter re-export ([#242](https://github.com/awslabs/metrique/pull/242))

## [0.1.20](https://github.com/awslabs/metrique/compare/metrique-v0.1.19...metrique-v0.1.20) - 2026-03-04

### Added

- add LocalFormat for human-readable local development metrics ([#213](https://github.com/awslabs/metrique/pull/213))

### Other

- Add example for global up/down counter ([#207](https://github.com/awslabs/metrique/pull/207))

## [0.1.19](https://github.com/awslabs/metrique/compare/metrique-v0.1.18...metrique-v0.1.19) - 2026-02-18

### Added

- Add runtime-wide time source override for tokio ([#206](https://github.com/awslabs/metrique/pull/206))

- Histogram fields can now be aggregated across structs using `#[aggregate(strategy = Histogram<T>)]`. This enables merging latency distributions from multiple sources (e.g. fan-out shards) into a single combined distribution. ([#204](https://github.com/awslabs/metrique/pull/204))

## [0.1.18](https://github.com/awslabs/metrique/compare/metrique-v0.1.17...metrique-v0.1.18) - 2026-02-14

### Other

- Add Debug derive to SetEntryDimensions ([#202](https://github.com/awslabs/metrique/pull/202))

## [0.1.17](https://github.com/awslabs/metrique/compare/metrique-v0.1.16...metrique-v0.1.17) - 2026-02-07

### Fixed

- Fix issues with `#[derive(Debug)]` on metrics entries ([#200](https://github.com/awslabs/metrique/pull/200))

## [0.1.16](https://github.com/awslabs/metrique/compare/metrique-v0.1.15...metrique-v0.1.16) - 2026-02-01

### Added

- impl Debug for Slot, SlotGuard, and AppendAndCloseOnDrop ([#194](https://github.com/awslabs/metrique/pull/194))

- Add `set_test_sink_on_current_tokio_runtime` to simplify testing with Tokio ([#193](https://github.com/awslabs/metrique/pull/193))

### Other

## [0.1.15](https://github.com/awslabs/metrique/compare/metrique-v0.1.14...metrique-v0.1.15) - 2026-01-16

### Fixes 

- Fix docs.rs build, add docs.rs build check script and CI job ([#188](https://github.com/awslabs/metrique/pull/188))

## [0.1.14](https://github.com/awslabs/metrique/compare/metrique-v0.1.13...metrique-v0.1.14) - 2026-01-15

### Added

- Add support for `#[aggregate]` and aggregation ([#158](https://github.com/awslabs/metrique/pull/158)). This is a major new feature—you can find lots of docs and examples in the `metrique-aggregation` package. `#[aggregate]` allows you to take your existing unit-of-work metrics and aggregate them, potentially across multiple different sets of dimensions.

```rust
#[aggregate]
#[metrics]
struct BackendCall {
    #[aggregate(strategy = Sum)]
    requests_made: usize,

    #[aggregate(strategy = Histogram<Duration>)]
    #[metrics(unit = Millisecond)]
    latency: Duration,

    #[aggregate(strategy = Sum)]
    errors: u64,
}
```

### Other

- Enable rustdoc-scrape-examples for docs.rs ([#181](https://github.com/awslabs/metrique/pull/181))
- remove vestigial test sink .as_u64() / add_f64() calls ([#171](https://github.com/awslabs/metrique/pull/171))

## [0.1.13](https://github.com/awslabs/metrique/compare/metrique-v0.1.12...metrique-v0.1.13) - 2026-01-10

### Added

- Add support for lifetimes with `#[metrics]` ([#169](https://github.com/awslabs/metrique/pull/169))

- Add support for entry enums ([#156](https://github.com/awslabs/metrique/pull/156))

Entry enum example:
```rust
#[metrics(tag(name = "Operation"), subfield)]
enum OperationMetrics {
    Read(#[metrics(flatten)] ReadMetrics),
    Delete {
        key_count: usize,
    },
}

#[metrics(rename_all = "PascalCase")]
struct MyMetrics {
  operation: MyOperationMetrics,
  success: bool, // this could be an enum too, if you wanted detailed success/failure metrics!
  request_id: String,
}

#[metrics(subfield)]
struct ReadMetrics {
  files_read: usize
}

// you would normally compose this gradually, and append on drop
let a_metric = RequestMetrics {
  success: true,
  request_id: "my_request".to_string(),
  operation: OperationMetrics::Delete { key_count: 5 }
};
// Values: { "RequestId": "my_request", "Operation": "Delete" }
// Metrics: { "Success": 1, "KeyCount": 5 }
```

- [more information on entry enums](https://docs.rs/metrique/latest/metrique/unit_of_work/attr.metrics.html#enums)
- [longer-form entry enum example](metrique/examples/enums.rs)



## [0.1.12](https://github.com/awslabs/metrique/compare/metrique-v0.1.11...metrique-v0.1.12) - 2026-01-06

### Added

- [**breaking**] Add MetricMap wrapper for better error messages in test_util ([#157](https://github.com/awslabs/metrique/pull/157)). The change is technically a breaking change since it alters the type of a public API, however, it is 
  very unlikely to break actual code.

### Fixed

- Add scaling factor to ExponentialAggregationStrategy. This improves storage resolution for durations <1ms and numeric values <1. ([#148](https://github.com/awslabs/metrique/pull/148))

### Other

- [macros] Reorganization, CloseValue diagnostic improvement ([#162](https://github.com/awslabs/metrique/pull/162))
- *(docs)* clarify that you can also bring your own format with the Format trait ([#98](https://github.com/awslabs/metrique/pull/98))

## [0.1.11](https://github.com/awslabs/metrique/compare/metrique-v0.1.10...metrique-v0.1.11) - 2025-12-19

### Breaking Changes

- forbid `.` in non-exact prefixes ([#138](https://github.com/awslabs/metrique/pull/138))
- forbid root-level prefixes that do not end with a delimiter ([#138](https://github.com/awslabs/metrique/pull/138))

## [0.1.10](https://github.com/awslabs/metrique/compare/metrique-v0.1.9...metrique-v0.1.10) - 2025-12-15

### Added

- generate docs for guard and handle types ([#134](https://github.com/awslabs/metrique/pull/134))

### Other

- Update MSRV to 1.89, darling to 0.23 ([#135](https://github.com/awslabs/metrique/pull/135))
- Add support for sampling, improve docs ([#133](https://github.com/awslabs/metrique/pull/133))
- add examples for WithDimensions, clean up metrics docs ([#132](https://github.com/awslabs/metrique/pull/132))

## [0.1.9](https://github.com/arielb1/metrique-fork/compare/metrique-v0.1.8...metrique-v0.1.9) - 2025-12-04

### Added

- show useful error information on validation error with no tracing ([#129](https://github.com/awslabs/metrique/pull/129))

## [0.1.8](https://github.com/awslabs/metrique/compare/metrique-v0.1.7...metrique-v0.1.8) - 2025-11-23

### Other

- support older versions of Tokio (from Tokio 1.38) ([#126](https://github.com/awslabs/metrique/pull/126))
- declare MSRV for all crates ([#126](https://github.com/awslabs/metrique/pull/126))

## [0.1.7](https://github.com/awslabs/metrique/compare/metrique-v0.1.6...metrique-v0.1.7) - 2025-11-12

### Added

- implement `#[metrics(explicit_prefix)]` which is not inflected ([#122](https://github.com/awslabs/metrique/pull/122))

### Fixed

- Timer::stop should be idempotent ([#115](https://github.com/awslabs/metrique/pull/115))
- add track_caller to GlobalEntrySink::sink ([#123](https://github.com/awslabs/metrique/pull/123))

### Other

- improve documentation (several PRs)
- replace doc_auto_cfg with doc_cfg ([#111](https://github.com/awslabs/metrique/pull/111))

### Breaking changes

- reserve `ForceFlushGuard: !Unpin` ([#119](https://github.com/awslabs/metrique/pull/119))

## [0.1.6](https://github.com/awslabs/metrique/compare/metrique-v0.1.5...metrique-v0.1.6) - 2025-09-19

### Added

- *(emf)* allow selecting a log group name ([#107](https://github.com/awslabs/metrique/pull/107))
- *(test-util)* derive Clone and Debug for TestEntrySink ([#92](https://github.com/awslabs/metrique/pull/92))

### Fixed

- *(docs)* properly set global dimension in emf module docs ([#97](https://github.com/awslabs/metrique/pull/97))

### Other

- Add "why" section to the README ([#101](https://github.com/awslabs/metrique/pull/101))
- *(macro)* minor field description enhancement ([#96](https://github.com/awslabs/metrique/pull/96))

## `metrique-service-metrics` - [0.1.5](https://github.com/awslabs/metrique/compare/metrique-service-metrics-v0.1.4...metrique-service-metrics-v0.1.5) - 2025-08-25

### Fixes
- allow `metrique::writer::Entry` to work without a metrique-writer import
- make `metrique/test-util` depend on `metrique-metricsrs/test-util`

## `metrique-core` - [0.1.5](https://github.com/arielb1/metrique-fork/compare/metrique-core-v0.1.4...metrique-core-v0.1.5) - 2025-08-20

### Added
- Added DevNullSink ([#85](https://github.com/awslabs/metrique/commit/c5d6c19ac4d48a80523ea34c015b1baf9d762714)),
  which is an EntrySink that drops all entries.

### Breaking Changes
- moved `metrique_writer::metrics` to `metrique_metricsrs` / `metrique::metrics_rs` ([#88](https://github.com/awslabs/metrique/pull/88))
- Changed the `metrics` API to support multiple metrics.rs versions. You will need to pass
  `dyn metrics::Recorder` type parameters to enable detecting the right metrics.rs version - see
  the function docs for more details. ([#86](https://github.com/awslabs/metrique/commit/057ad73fb7a2f0989c9fd74c55b9596611ba05a0)).
- Changed `FlushWait` to be `Send + Sync`, which will break if you called `FlushWait::from_future`
  with a future that is not `Send + Sync`.

### Other
- updated the following local packages: metrique-writer-core

## `metrique` - 0.1.4

### Added
- Add support for prefixes to flattened fields ([#65](https://github.com/awslabs/metrique/pull/65)). This enables patterns like:
  ```rust
  #[metrics]
  struct RequestMetrics {
      #[metrics(flatten, prefix = "a_")]
      operation_a: OperationMetrics,
      #[metrics(flatten, prefix = "b_")]
      operation_b: OperationMetrics,
  }

- `metrique` now re-exports `metrique-writer` behind the `metrique::writer` module. This removes the need to add a separate dependency on `metrique_writer`. ([#76](https://github.com/awslabs/metrique/pull/76))
- Added an `emit` method on `Instrumented`

## `metrique-writer` - [0.1.4](https://github.com/awslabs/metrique/compare/metrique-writer-v0.1.3...metrique-writer-v0.1.4) - 2025-08-13

### Other
- Reexport metrique_writer from metrique ([#76](https://github.com/awslabs/metrique/pull/76))
- Make metrique-writer enable metrique-writer-core test-util ([#80](https://github.com/awslabs/metrique/pull/80))
- add docsrs cfg and clean docs ([#73](https://github.com/awslabs/metrique/pull/73))

## `metrique-writer-macro` - [0.1.1](https://github.com/awslabs/metrique/compare/metrique-writer-macro-v0.1.0...metrique-writer-macro-v0.1.1) - 2025-08-13

### Other
- Reexport metrique_writer from metrique ([#76](https://github.com/awslabs/metrique/pull/76))
- add docsrs cfg and clean docs ([#73](https://github.com/awslabs/metrique/pull/73))

## `metrique-macro` - [0.1.2](https://github.com/awslabs/metrique/compare/metrique-macro-v0.1.1...metrique-macro-v0.1.2) - 2025-08-13

### Other
- Reexport metrique_writer from metrique ([#76](https://github.com/awslabs/metrique/pull/76))
- Add support for prefixes to flattened fields ([#65](https://github.com/awslabs/metrique/pull/65))
- add docsrs cfg and clean docs ([#73](https://github.com/awslabs/metrique/pull/73))

## `metrique-core` - [0.1.4](https://github.com/awslabs/metrique/compare/metrique-core-v0.1.3...metrique-core-v0.1.4) - 2025-08-13

### Other
- Reexport metrique_writer from metrique ([#76](https://github.com/awslabs/metrique/pull/76))
- Add support for prefixes to flattened fields ([#65](https://github.com/awslabs/metrique/pull/65))
- add docsrs cfg and clean docs ([#73](https://github.com/awslabs/metrique/pull/73))

## `metrique-writer-core` - [0.1.4](https://github.com/awslabs/metrique/compare/metrique-writer-core-v0.1.3...metrique-writer-core-v0.1.4) - 2025-08-13

### Fixed
- try to fix rustdoc ([#78](https://github.com/awslabs/metrique/pull/78))

### Other
- Reexport metrique_writer from metrique ([#76](https://github.com/awslabs/metrique/pull/76))
- add docsrs cfg and clean docs ([#73](https://github.com/awslabs/metrique/pull/73))

## [0.1.3](https://github.com/awslabs/metrique/compare/metrique-core-v0.1.2...metrique-core-v0.1.3) - 2025-08-12

### Added

- Added global `metrique::ServiceMetrics` entry sink

### Breaking Fixes

- mark ThreadLocalTestSinkGuard as !Send + !Sync

## [0.1.2](https://github.com/arielb1/metrique-fork/compare/metrique-core-v0.1.1...metrique-core-v0.1.2) - 2025-08-06

### Added

- update the reporters for metrics.rs to accept `AnyEntrySink` as well as `impl EntryIoStream`

### Fixes

- fixed a bug in the macro-generated doctests of `global_entry_sink`

## [0.1.1](https://github.com/awslabs/metrique/compare/metrique-writer-core-v0.1.0...metrique-writer-core-v0.1.1) - 2025-08-05

### Added

- allow `WithDimensions` and `ForceFlag` support for entries
- breaking change: clean up `CloseValue`/`CloseValueRef`. If you previously implemented `CloseValueRef`, you should now implement `CloseValue for &'_ T`
- separate `#[metrics(no_close)]` from `#[metrics(flatten_entry)]`.
  The old `#[metrics(flatten_entry)]` is now `#[metrics(flatten_entry, no_close)]`.
- allow using `ForceFlag` for `CloseValue`. This allows setting things like `emf::HighResolution<Value>`
- support `#[metrics(value)]` and `#[metrics(value(string))]`. These reduce one of the main reasons to implement `CloseValue` directly: using a enum as a string value in your metric:
    ```rust
    #[metric(value(string))]
    enum ActionType {
      Create,
      Update,
      Delete
    }
    ```
