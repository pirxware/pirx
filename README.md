# pirx

**Execution profiler for fault-tolerant quantum computing.**

[![CI](https://img.shields.io/github/actions/workflow/status/pirxware/pirx/ci.yml?branch=master&style=flat-square&label=CI)](https://github.com/pirxware/pirx/actions/workflows/ci.yml)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json&style=flat-square)](https://codspeed.io/pirxware/pirx)
![License](https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square)
![MSRV](https://img.shields.io/badge/rust-1.88%2B-orange?style=flat-square)

<sub>Named after [Pilot Pirx](https://en.wikipedia.org/wiki/Tales_of_Pirx_the_Pilot) from Stanisław Lem's stories — the methodical engineer who traces what actually happened vs. what the instruments claimed.</sub>

[Security](SECURITY.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

---

GPU computing had to build profiling tools before optimization tools could exist — Nsight Systems, perf, VTune came first, then the compilers learned to use their output. You can't optimize what you can't observe. Fault-tolerant quantum computing has no profiling layer. Every resource estimation tool gives you a single number — total qubits, total runtime — and leaves you guessing where the bottleneck is.

Pirx fills that gap. It is the discrete-event simulation engine that takes a compiled quantum circuit and a hardware model, and produces a temporal execution profile — showing exactly what happens, cycle by cycle, when a fault-tolerant quantum computation runs.

> [!IMPORTANT]
> **Under active development.** The API and manifest format may have breaking changes until v1.0. Pin to a specific commit for production use.

## The problem

Resource estimators tell you: *"4 million qubits, 16 hours."*

They model factory throughput as a steady-state average: `runtime = total_T / (factories x throughput)`. This is like sizing a factory based on annual demand without looking at weekly peaks. Three phenomena that materially affect execution are invisible:

- **Non-uniform demand.** Algorithms don't consume magic states at a constant rate. QFT butterfly stages are T-dense; Clifford stretches are T-free. The temporal demand pattern determines whether factories stall or qubits idle.
- **Stochastic production and consumption.** Cultivation has exponentially distributed production times (mean ≈ 26 cycles at d=17, long tail). Every T-gate injection fails with 50% probability, inserting S-gate fixups that reshape the schedule at runtime. Awasthi et al. (2026) demonstrated this dual effect systematically mischaracterizes execution costs — up to 2.5× for cultivation.
- **Shifting bottlenecks.** Different execution phases are constrained by different resources: factory throughput, routing contention, buffer capacity. No existing tool localizes bottlenecks in time.

Researchers cannot answer: *"where is my execution bottleneck, and what should I optimize?"*

## What Pirx does

Pirx tells you *where* those 16 hours go — which cycles are factory-bound, which are routing-bound, what happens when injection errors reshape the schedule, and which hardware parameter you should change first.

**Working today:**

- **Discrete-event simulation engine** — cycle-accurate execution of fault-tolerant circuits with DAG-based dependency scheduling, deterministic reproducibility (same seed = identical trace), and injection error recovery with fixup node insertion
- **Stochastic factory models** — cultivation (exponential service time) and distillation (multi-round with per-round abort probability), both driven by an explicit seeded RNG
- **Magic state buffer dynamics** — finite-capacity buffer with cold/warm start, FIFO stall queue, and per-gate wait-time accounting
- **Post-hoc trace analysis** — single O(n) pass producing time-bucketed execution profiles: factory utilization, buffer occupancy, bottleneck classification (factory-throughput / routing-contention / balanced), stall records, injection error counts, and critical-path extension
- **Pluggable hardware models** — TOML-specified, validated at load time; surface code, color code, and qLDPC families; ships with two reference models (d=17 cultivation, d=17 distillation)
- **Property-based testing** — proptest-driven invariant checking (determinism, monotonicity, buffer bounds, factory scaling) plus Criterion/CodSpeed benchmarks

**Planned:**

- Sensitivity analysis — Sobol indices to reveal which hardware parameter dominates runtime variance
- Framework adapters — OpenQASM 3, FTCircuitBench
- Cross-architecture comparison — same circuit profiled across code families
- Full CLI with JSON report output

## Architecture

Six crates with strict dependency direction:

```
pirx-ir         Framework-agnostic circuit representation (Profiler IR)
pirx-hw         Hardware model TOML types, parsing, and validation
pirx-core       DES engine, factory models, buffer, trace collection, analysis
pirx-adapters   Framework converters (planned: OpenQASM 3, FTCircuitBench)
pirx-cli        CLI binary (scaffold)
pirx-testkit    Shared test fixtures for the workspace (dev only)
```

The core treats FTQC execution as a production system: magic-state factories are stochastic producers, the algorithm's T-gate sequence is the consumer, and the buffer between them is inventory. The math is queueing theory, critical-path scheduling, and global sensitivity analysis — well-established engineering disciplines applied to a domain that hasn't adopted them yet.

## Hardware model

```toml
[meta]
name = "surface_code_d17_cultivation"

[qec]
code_type = "surface_code"
code_distance = 17
physical_error_rate = 1e-3

[timing]
cycle_time_us = 1.0

[factory]
type = "cultivation"
count = 12
lambda_raw = 0.00227
fault_distance = 3

[injection]
error_probability = 0.5

[routing]
model = "scalar"
overhead_fraction = 0.5

[buffer]
capacity = 8
```

## Development

```bash
make setup    # install git hooks
make ci       # fmt + clippy + test + audit — run before pushing
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development guide.

## Supply chain security

Every CI run enforces:

- **SHA-pinned actions** — all GitHub Actions referenced by commit SHA, not mutable tags
- **7-day dependency quarantine** — any crate published less than 7 days ago fails the build
- **cargo-deny** — license allowlist, advisory database, source restrictions, duplicate detection
- **cargo-audit** — RustSec advisory database checks
- **CodeQL** — static analysis of workflow definitions
- **Signed releases** — minisign signatures + SHA-256 checksums + SLSA build provenance

## License

Apache-2.0
