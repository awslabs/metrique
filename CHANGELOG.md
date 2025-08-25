# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
