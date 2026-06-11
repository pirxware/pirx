# Contributing to Pirx

Pirx is an FTQC execution profiler. Contributions should meet the same engineering bar the tool itself targets: production-grade, zero-allocation hot paths, every line earns its place.

## Reporting bugs and security issues

**Security vulnerabilities** — do not open a public issue. See [SECURITY.md](SECURITY.md).

**Bugs** — open a GitHub issue with your Pirx version (`pirx --version`), Rust version (`rustc --version`), platform, and minimal reproduction steps.

**Feature requests** — open an issue describing the use case before writing code.

## Development setup

**Prerequisites:** Rust 1.88+ (`rustup update stable`), `make`, and `cargo-deny`:

```bash
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

## Running checks

```bash
make test       # unit + integration + property tests
make check      # cargo fmt --check + clippy -D warnings
make audit      # cargo deny check (advisories, licenses, bans, sources)
make ci         # all of the above — run before pushing
```

## Making changes

1. Branch from `main`: `git checkout -b feat/my-feature`.
2. Make changes. Add tests for non-trivial behavior.
3. `make fmt` to auto-format.
4. `make ci` to verify.
5. Commit using conventional commits (imperative present tense, ≤72 chars).
6. Open a PR against `main`.

## Crate boundaries

These are hard constraints — violations are bugs:

- **pirx-ir** — never references any external quantum framework
- **pirx-core** — never parses circuits directly, only consumes `ProfilerCircuit` from pirx-ir
- **pirx-adapters** — each adapter is independent, no shared state between adapters
- **pirx-hw** — TOML is the only config format

## Code standards

- `unsafe` is forbidden workspace-wide (`forbid(unsafe_code)`)
- `unwrap()` and `expect()` are denied outside tests
- Zero allocations in the simulation hot loop after `Engine::new()`
- All randomness via explicit `StdRng` parameter — same seed = same trace
- Every dependency justified in the PR that adds it

## Testing

No mocks. Test real behavior with real circuits. Property tests (`proptest`) for invariants. Criterion benchmarks for hot paths.

## License

Apache-2.0. By contributing, you agree that your contributions will be licensed under the same license.
