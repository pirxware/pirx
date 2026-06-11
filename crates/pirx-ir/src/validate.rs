//! Validation of Profiler IR circuits.

use crate::circuit::ProfilerCircuit;

/// Errors detected during IR validation.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Circuit contains no operations")]
    EmptyCircuit,

    #[error("Dependency references non-existent operation ID {0}")]
    DanglingDependency(u64),

    #[error("Operation {op_id} references qubit {qubit_id} beyond qubit_count {qubit_count}")]
    InvalidQubit {
        op_id: u64,
        qubit_id: u32,
        qubit_count: u32,
    },

    #[error("Dependency graph contains a cycle")]
    CyclicDag,
}

/// Validate that a [`ProfilerCircuit`] is well-formed.
pub fn validate(_circuit: &ProfilerCircuit) -> Result<(), ValidationError> {
    // TODO(#1): implement full validation (acyclicity, qubit bounds, dep refs)
    Ok(())
}
