//! Adapter error types.

use pirx_ir::validate::ValidationError;

/// Errors produced by the OpenQASM 3 adapter.
#[derive(Debug, thiserror::Error)]
pub enum OpenQasmError {
    #[error("OpenQASM parse error: {0}")]
    Parse(String),

    #[error("symbolic parameter '{name}' in gate '{gate}' — bind all parameters before profiling")]
    SymbolicParameter { gate: String, name: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Validation(#[from] ValidationError),
}

/// Errors produced by the tket JSON adapter.
#[derive(Debug, thiserror::Error)]
pub enum TketJsonError {
    #[error("tket JSON parse error: {0}")]
    Parse(String),

    #[error("unsupported tket operation '{op_type}' — decompose or remove before profiling")]
    UnsupportedOperation { op_type: String },

    #[error("symbolic parameter '{param}' in gate '{gate}' — bind all parameters before profiling")]
    SymbolicParameter { gate: String, param: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Validation(#[from] ValidationError),
}
