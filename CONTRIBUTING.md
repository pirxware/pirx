# Contributing to Pirx

Pirx is an FTQC execution profiler. Contributions should meet the same engineering bar the tool itself targets: production-grade, zero-allocation hot paths, every line earns its place.

## Reporting Issues

**Security vulnerabilities** — do not open a public issue. See [SECURITY.md](SECURITY.md).

**Bugs** — open a GitHub issue with your Pirx version (`pirx --version`), Rust version (`rustc --version`), platform, and minimal reproduction steps.

**Feature requests** — open an issue describing the use case before writing code.

## Development Setup

**Prerequisites:** Rust stable 1.88+, Rust nightly (for `rustfmt`), `make`, and `cargo-deny`:

```bash
rustup update stable
rustup toolchain install nightly
cargo install cargo-deny
```

**First-time setup:**

```bash
git clone https://github.com/pirxware/pirx.git
cd pirx
make setup    # installs git hooks
cargo build
cargo test
```

**Faster linker (optional, Linux):**

```bash
sudo apt install clang mold
```

Add to `~/.cargo/config.toml`:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

## Running Checks

| Command      | What it does                                          |
|--------------|-------------------------------------------------------|
| `make test`  | Unit + integration + property tests                   |
| `make check` | `cargo +nightly fmt --check` + `clippy -D warnings`  |
| `make audit` | `cargo deny check` (advisories, licenses, bans)       |
| `make bench` | Criterion benchmarks                                  |
| `make ci`    | `check` + `test` + `audit` — run before pushing       |
| `make fmt`   | Auto-format + auto-fix clippy lints                    |

## Making Changes

1. Branch from `master`: `git checkout -b feat/my-feature`
2. Make changes. Add tests for non-trivial behavior.
3. `make fmt` to auto-format.
4. `make ci` to verify everything passes.
5. Commit using [Conventional Commits](https://www.conventionalcommits.org/) (imperative present tense, 72 char subject line).
6. Open a PR against `master`.

## Crate Boundaries

These are hard constraints — violations are bugs:

| Crate | Constraint |
|-------|------------|
| **pirx-ir** | Never references any external quantum framework |
| **pirx-core** | Never parses circuits directly; only consumes `ProfilerCircuit` from pirx-ir |
| **pirx-adapters** | Each adapter is independent; no shared state between adapters |
| **pirx-hw** | TOML is the only config format |

## Code Standards

- `unsafe` is forbidden workspace-wide (`#![forbid(unsafe_code)]`)
- `unwrap()` and `expect()` are denied outside `#[cfg(test)]`
- Zero allocations in the simulation hot loop after `Engine::new()`
- All randomness via explicit `ChaCha12Rng` parameter — same seed = same trace
- Every dependency justified in the PR that adds it
- No async runtime, no proc macros at runtime

## Testing

No mocks. Test real behavior with real circuits.

- **Unit tests:** pure functions, same file
- **Integration tests:** full engine runs on known circuits, assert trace properties
- **Property tests:** `proptest` invariants — determinism, monotonic traces, bounded buffers
- **Benchmarks:** Criterion + CodSpeed for hot paths

## License

Apache-2.0. By contributing, you agree that your contributions will be licensed under the same license.
