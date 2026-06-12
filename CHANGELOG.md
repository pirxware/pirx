# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added

#### pirx-ir
- Profiler IR circuit representation — operations, dependencies, qubit assignments, circuit metadata
- IR validation (Kahn's acyclicity check, duplicate op IDs, dangling dependencies, qubit range checks) returning `ValidatedCircuit` proof token
- Measurement hooks and conditional activation for adaptive circuits (repeat-until-success, feedforward branching)
- Grid positions for distance-aware routing models
- Rotation operations (`OpKind::Rotation`) with f64 angle
- `initially_active` flag for pre-allocated conditional ops

#### pirx-hw
- Hardware model TOML specification, parsing, and validation
- Three QEC code families: surface code, color code, qLDPC
- Two factory types: cultivation (exponential service time), distillation (multi-round with per-round abort)
- RzSynthesis factory type (schema only, not yet implemented in engine)
- Distillation protocols: 15-to-1 and CCZ-to-2T
- Routing models: scalar (overhead fraction) and graph (schema only)
- Injection error parameters (error probability, fixup cost in cycles)
- Buffer configuration with preload support for warm-start simulations
- Comprehensive domain validation (code distance parity, probability ranges, positive rates)
- Two reference hardware models: `surface_code_d17_cultivation.toml`, `surface_code_d17_distillation.toml`

#### pirx-core
- Discrete-event simulation engine with DAG-based dependency scheduling
- DAG construction from `ValidatedCircuit` with rotation-angle deduplication (u16 index, max 65535 distinct angles)
- Injection error model — stochastic injection failures insert fixup nodes into the DAG at runtime
- Magic state buffer (fixed-capacity, enqueue/dequeue, cold and warm start)
- Cultivation factory model (exponential service time scaled by code distance)
- Distillation factory model (multi-round with independent per-round abort probability)
- Deterministic min-heap event queue with sequence-number tie-breaking
- Trace collection system — 16 event kinds, 24 bytes per event, append-only, pre-allocated
- Post-hoc trace analysis (`ProfileAnalyzer`) — single O(n) pass producing `ExecutionProfile`
- Time-bucketed bottleneck classification (None, FactoryThroughput, RoutingContention, Balanced)
- Factory utilization tracking, stall records, injection error counting, critical-path extension measurement
- Pluggable `ReadyQueue` trait with FIFO default implementation
- Full deterministic reproducibility — same seed + same inputs = identical trace

#### pirx-cli
- CLI scaffold with `profile` subcommand (argument parsing only)

#### pirx-adapters
- Crate scaffold (no adapters implemented yet)

#### pirx-testkit
- Shared test fixtures — hardware model builders, circuit builders, deterministic distillation helper

#### Testing & CI
- Property-based tests with proptest (determinism, trace monotonicity, buffer capacity bounds, factory scaling invariant, Clifford-no-stall invariant)
- Integration tests: single Clifford, immediate T-gate serve, stall-then-serve, dependency chain ordering, parallel scheduling, cross-factory determinism, injection fixup lifecycle
- Criterion benchmarks: `engine_new`, `engine_run`, `engine_step`, `trace_analysis` — parameterized by circuit size
- CodSpeed integration for CI benchmark tracking
- CI pipeline: lint (rustfmt + clippy), test, audit (cargo-deny, cargo-audit, 7-day dependency quarantine), CodeQL
- Release pipeline: 5-target cross-compilation, reproducible archives, SHA-256 checksums, SLSA build provenance, SBOM generation, minisign signatures
- `deny.toml` for cargo-deny license and advisory policy
- `Cross.toml` for cross-compilation targets
- Dependabot for Cargo and GitHub Actions dependency updates
