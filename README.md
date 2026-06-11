# pirx

**Execution profiler for fault-tolerant quantum computing.**

[![CI](https://img.shields.io/github/actions/workflow/status/m2papierz/pirx/ci.yml?style=flat-square&label=CI)](https://github.com/m2papierz/pirx/actions/workflows/ci.yml)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json&style=flat-square)](https://codspeed.io/m2papierz/pirx)
![License](https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square)
![Rust](https://img.shields.io/badge/rust-1.95%2B-orange?style=flat-square)

<sub>Named after [Pilot Pirx](https://en.wikipedia.org/wiki/Tales_of_Pirx_the_Pilot) from Stanisław Lem's stories - the methodical engineer who traces what actually happened vs. what the instruments claimed.</sub>

[Security](SECURITY.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

---

GPU computing had to build profiling tools (Nsight, perf) before optimization tools could exist. You can't optimize what you can't observe. Fault-tolerant quantum computing has no profiling layer. Every resource estimation tool gives you a single number — total qubits, total runtime — and leaves you guessing where the bottleneck is.

Pirx fills that gap. It is the performance engineering platform for FTQC: a discrete-event simulator that takes a compiled quantum circuit and a hardware model, and produces a temporal execution profile — showing you exactly what happens, cycle by cycle, when a fault-tolerant quantum computation runs.

> [!IMPORTANT]
> **Under active development.** Not yet ready for use.

## Goal
 
Resource estimators tell you: *"4M qubits, 16 hours."*
 
Pirx aims to tell you *where* those 16 hours go — which cycles are factory-bound, which are routing-bound, what happens when injection errors reshape the schedule, and which hardware parameter you should change first.
 
Specifically, the target capabilities are:
 
- **Temporal bottleneck localization** — not "it's slow" but "these cycles are factory-bound, those are routing-bound"
- **Stochastic execution dynamics** — magic state cultivation, injection errors, distillation aborts
- **Sensitivity analysis** — which hardware parameter dominates runtime variance
- **Cross-architecture comparison** — same circuit profiled on different hardware models, side by side
- **Pluggable hardware models** — TOML specs shareable alongside papers

## Architecture
 
Five crates with strict dependency direction:
 
```
pirx-ir         Framework-agnostic circuit representation (Profiler IR)
pirx-hw         Hardware model TOML types and parsing
pirx-core       DES engine, factory models, trace collection, analysis
pirx-adapters   Framework converters (OpenQASM 3, FTCircuitBench, ...)
pirx-cli        CLI binary
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

[buffer]
capacity = 8
```

## Development

```bash
make setup    # install git hooks
make ci       # fmt + clippy + test + audit — run before pushing
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development guide.

## License

Apache-2.0
