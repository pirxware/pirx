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
