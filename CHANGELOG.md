# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
