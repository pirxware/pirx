//! Framework adapters — convert external circuit representations
//! into Profiler IR ([`pirx_ir::circuit::ProfilerCircuit`]).
//!
//! Each adapter is independent. No shared state between adapters.
//! Adapters use external frameworks as-is — no forking, no patching.

pub mod error;
pub mod openqasm;
#[cfg(feature = "tket-json")]
pub mod tket_json;

pub use error::OpenQasmError;
#[cfg(feature = "tket-json")]
pub use error::TketJsonError;
pub use openqasm::{from_qasm_file, from_qasm_str};
#[cfg(feature = "tket-json")]
pub use tket_json::{from_tket_json_file, from_tket_json_str};
