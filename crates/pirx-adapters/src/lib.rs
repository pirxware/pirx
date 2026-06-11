//! Framework adapters — convert external circuit representations
//! into Profiler IR ([`pirx_ir::circuit::ProfilerCircuit`]).
//!
//! Each adapter is independent. No shared state between adapters.
//! Adapters use external frameworks as-is — no forking, no patching.

// TODO: openqasm3, ftcircuitbench adapters
