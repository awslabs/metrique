# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
