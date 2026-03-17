# Fuzzing

Uses [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) with libFuzzer to stress formatter invariants.

## Targets

- `fuzz_json`: If formatting returns `Ok(())`, output must be exactly one valid, newline-terminated JSON object. Tests both regular and sampled paths.
- `fuzz_emf`: If formatting returns `Ok(())`, each emitted line must be a valid JSON object. Tests both regular and sampled paths with EMF-specific flag modes.

Both targets format multiple entries through the same formatter instance to exercise state reuse.

## Run Locally

Requires Rust nightly and `cargo-fuzz` (`cargo install cargo-fuzz`).

```bash
cargo +nightly fuzz run fuzz_json -- -max_total_time=60 -print_coverage=1
cargo +nightly fuzz run fuzz_emf -- -max_total_time=60 -print_coverage=1
```

## Reproduce a Crash

```bash
cargo +nightly fuzz run fuzz_json fuzz/artifacts/fuzz_json/<crash-file>
```

Minimize locally with:

```bash
cargo +nightly fuzz cmin fuzz_json
cargo +nightly fuzz cmin fuzz_emf
```

## CI

- **PRs**: 1-minute smoke test per target.
- **Nightly**: 5-minute run per target (schedule + workflow_dispatch).
