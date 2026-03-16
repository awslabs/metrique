# Fuzzing

Uses [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) with libFuzzer to stress formatter invariants.

## Targets

- `fuzz_json`: If formatting returns `Ok(())`, output must be exactly one valid, newline-terminated JSON object. Tests both regular and sampled paths.
- `fuzz_emf`: If formatting returns `Ok(())`, each emitted line must be a valid JSON object. Tests both regular and sampled paths with EMF-specific flag modes.

Both targets format two entries through the same formatter instance to exercise state reuse. Additional semantic invariants may be added in the future.

## Run Locally

Requires Rust nightly and `cargo-fuzz` (`cargo install cargo-fuzz`).

```bash
cargo +nightly fuzz run fuzz_json -- -max_total_time=60
cargo +nightly fuzz run fuzz_emf -- -max_total_time=60
```

## Reproduce a Crash

```bash
cargo +nightly fuzz run fuzz_json fuzz/artifacts/fuzz_json/<crash-file>
```

Then fix the bug, add a deterministic regression test, and delete the reproducer.

## Corpus

`fuzz/corpus` is git-ignored. Do not commit the evolving corpus.

Minimize locally with:

```bash
cargo +nightly fuzz cmin fuzz_json
cargo +nightly fuzz cmin fuzz_emf
```

## CI

Nightly GitHub Actions workflow:

1. Restores corpus from cache
2. Runs both targets (5 min each)
3. Minimizes corpus via `cargo fuzz cmin`
4. Saves corpus back to cache

Corpus cache uses branch-scoped daily buckets with weekly/branch fallback, so coverage accumulates across runs without committing corpus files.
