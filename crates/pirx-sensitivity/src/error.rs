//! Sensitivity analysis error types.

use pirx_core::{EngineError, monte_carlo::MonteCarloError};
use pirx_hw::HardwareModelError;
use thiserror::Error;

/// Errors from parameter sensitivity analysis.
#[derive(Debug, Error)]
pub enum SensitivityError {
    #[error("unknown parameter: {0}")]
    UnknownParameter(String),

    #[error("parameter '{param}' requires factory type '{expected}'")]
    FactoryTypeMismatch {
        param: String,
        expected: &'static str,
    },

    #[error("parameter '{param}' requires routing type '{expected}'")]
    RoutingTypeMismatch {
        param: String,
        expected: &'static str,
    },

    #[error("invalid parameter range: {name} min={min} >= max={max}")]
    InvalidRange { name: String, min: f64, max: f64 },

    #[error("duplicate parameter: {0}")]
    DuplicateParameter(String),

    #[error("hardware validation failed after mutation: {0}")]
    HardwareValidation(#[from] HardwareModelError),

    #[error("engine error: {0}")]
    Engine(#[from] EngineError),

    #[error("monte carlo error: {0}")]
    MonteCarlo(#[from] MonteCarloError),

    #[error("parameter space is empty (no parameters to sweep)")]
    EmptyParameterSpace,

    #[error("dimension mismatch: expected {expected} values, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    #[error("config parse error: {0}")]
    ConfigParse(String),

    #[error("Morris levels must be even and >= 4, got {0}")]
    InvalidMorrisLevels(u32),

    #[error("Sobol n_samples must be a power of 2, got {0}")]
    NotPowerOfTwo(u32),

    #[error("Sobol n_samples must be >= 64, got {0}")]
    InsufficientSamples(u32),

    #[error("Morris trajectories must be >= 2, got {0}")]
    InsufficientTrajectories(u32),

    #[error("parameter '{param}' has non-integer bound for Integer kind: min={min}, max={max}")]
    NonIntegerBound { param: String, min: f64, max: f64 },

    #[error(
        "parameter '{param}' has non-odd or < 3 bound for OddInteger kind: min={min}, max={max}"
    )]
    NonOddBound { param: String, min: f64, max: f64 },

    #[error("parameter '{param}' has negative bound: min={min}")]
    NegativeBound { param: String, min: f64 },
}
