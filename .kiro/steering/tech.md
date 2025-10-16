# Technology Stack & Build System

## Build System
- **Cargo workspace**: Multi-crate workspace with 8 member crates
- **Rust Edition**: 2024
- **Resolver**: Version 3

## Core Dependencies
- **tokio**: Async runtime with sync primitives
- **serde/serde_json**: Serialization for EMF format
- **tracing**: Logging and instrumentation
- **crossbeam**: Lock-free data structures and utilities
- **chrono**: Date/time handling for timestamps

## Development Dependencies
- **trybuild**: Compile-fail tests for macro validation
- **insta**: Snapshot testing
- **tempfile**: Temporary file handling in tests
- **tokio-test**: Tokio testing utilities

## Common Commands

### Building
```bash
# Build entire workspace
cargo build

# Build specific crate
cargo build -p metrique

# Build with all features
cargo build --all-features
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p metrique

# Run UI tests (compile-fail tests)
cargo test -p metrique ui

# Run tests with specific features
cargo test --features test-util
```

### Documentation
```bash
# Generate docs for workspace
cargo doc --workspace --all-features

# Open docs in browser
cargo doc --open
```

### Examples
```bash
# Run specific example
cargo run --example unit-of-work-simple

# List all examples
cargo run --example
```

## Feature Flags
- `default`: Includes `service-metrics`
- `emf`: Re-exports EMF format support
- `test-util`: Testing utilities
- `service-metrics`: Global ServiceMetrics sink
- `private-test-util`: Internal testing (unstable)

## Workspace Structure
- **metrique**: Main user-facing crate
- **metrique-core**: Core traits and utilities
- **metrique-macro**: Procedural macros
- **metrique-writer**: Entry writing and sinks
- **metrique-writer-core**: Core writer abstractions
- **metrique-writer-format-emf**: EMF format implementation
- **metrique-timesource**: Time source abstractions
- **metrique-service-metrics**: Global metrics sink