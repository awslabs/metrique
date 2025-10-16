# Project Structure & Organization

## Workspace Layout

The project follows a multi-crate workspace pattern with clear separation of concerns:

```
metrique/                    # Main user-facing crate
├── src/
├── examples/               # Usage examples and patterns
├── tests/                  # Integration tests
└── docs/                   # Additional documentation

metrique-core/              # Core traits and utilities
├── src/
│   ├── lib.rs             # CloseValue, InflectableEntry traits
│   ├── atomics.rs         # Atomic metric types (Counter)
│   ├── close_value_impls.rs # Standard type implementations
│   └── namestyle.rs       # Name inflection utilities

metrique-macro/             # Procedural macros
├── src/
│   ├── lib.rs             # Main macro entry points
│   ├── entry_impl.rs      # #[metrics] macro implementation
│   └── snapshots/         # Macro output snapshots

metrique-writer*/           # Writer ecosystem (4 crates)
├── metrique-writer/        # High-level writer API
├── metrique-writer-core/   # Core abstractions
├── metrique-writer-format-emf/ # EMF format implementation
└── metrique-writer-macro/  # Writer-specific macros

metrique-timesource/        # Time source abstractions
metrique-service-metrics/   # Global metrics sink
```

## Code Organization Patterns

### Trait Hierarchy
- **CloseValue/CloseValueRef**: Core value closing traits
- **CloseEntry**: Trait alias for closable entries
- **InflectableEntry**: Name-inflectable metric entries
- **Entry**: Final entry trait for sinks

### Macro-Generated Code
- `#[metrics]` generates `<Name>Guard` type aliases
- Guard types are `AppendAndCloseOnDrop<Entry, Sink>`
- Automatic `append_on_drop()` method generation

### Testing Structure
- **Unit tests**: Alongside source in `src/`
- **Integration tests**: In `tests/` directories
- **UI tests**: Compile-fail tests in `tests/ui/`
- **Examples**: Comprehensive examples in `examples/`

## File Naming Conventions

### Source Files
- `lib.rs`: Main crate entry point with re-exports
- `<feature>.rs`: Feature-specific modules (emf.rs, timers.rs)
- `<concept>_impls.rs`: Trait implementations for standard types

### Test Files
- `<feature>.rs`: Feature-specific integration tests
- `ui.rs`: UI test runner
- `ui/fail/`: Compile-fail test cases with `.stderr` expected output

### Documentation
- `README.md`: Crate-level documentation
- `docs/`: Additional documentation files
- Inline docs: Comprehensive rustdoc comments

## Module Organization

### Public API Surface
- Re-export pattern: Core functionality re-exported from `lib.rs`
- Feature gates: Optional functionality behind feature flags
- Prelude pattern: Common imports grouped in modules

### Internal Structure
- Private modules for implementation details
- `#[doc(hidden)]` for internal but public items
- Clear separation between user API and internal traits

## Dependencies Management

### Workspace Dependencies
- Centralized version management in root `Cargo.toml`
- Consistent dependency versions across crates
- Feature flag coordination between crates

### Crate Relationships
- **metrique**: Depends on all other workspace crates
- **metrique-core**: Minimal dependencies, foundational traits
- **metrique-writer-***: Layered writer architecture
- Clear dependency direction (no circular dependencies)