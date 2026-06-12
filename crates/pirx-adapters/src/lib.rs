//! Framework adapters — convert external circuit representations
//! into Profiler IR ([`pirx_ir::circuit::ProfilerCircuit`]).
//!
//! Each adapter is independent. No shared state between adapters.
//! Adapters use external frameworks as-is — no forking, no patching.

pub mod error;
pub mod openqasm;

pub use error::OpenQasmError;
pub use openqasm::{from_qasm_file, from_qasm_str};
