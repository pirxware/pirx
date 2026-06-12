//! Engine configuration and error types.

use pirx_ir::circuit::{MeasurementHookId, OpId};
use thiserror::Error;

use crate::{dag::DagError, factory::FactoryError};

/// Configuration for [`Engine`](super::Engine) construction.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    /// RNG seed. Same seed + same inputs = identical trace, always.
    pub seed: u64,
    /// Maximum simulation cycles. `None` = run to completion.
    /// When hit, the engine stops and the trace records `truncated: true`.
    pub max_cycles: Option<u64>,
}

/// Errors that can occur during engine construction.
///
/// All validation happens inside [`Engine::new`](super::Engine::new). If `new`
/// returns `Ok`, the simulation is guaranteed to run to completion without error.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("too many distinct rotation angles: {0} (maximum 65535)")]
    TooManyRotationAngles(usize),

    #[error("hardware model has zero factories")]
    NoFactories,

    #[error("buffer capacity is zero")]
    ZeroBuffer,

    #[error("factory creation failed: {0}")]
    FactoryCreation(#[from] FactoryError),

    #[error("manhattan routing requires qubit_positions in circuit")]
    MissingQubitPositions,

    #[error("measurement hook {hook_id} references non-existent op {op_id}")]
    DanglingHookTarget {
        hook_id: MeasurementHookId,
        op_id: OpId,
    },

    #[error("internal DAG error: {0}")]
    Internal(String),
}

impl From<DagError> for EngineError {
    fn from(err: DagError) -> Self {
        match err {
            DagError::TooManyDistinctAngles(n) => Self::TooManyRotationAngles(n),
            DagError::Internal(msg) => Self::Internal(msg),
        }
    }
}
