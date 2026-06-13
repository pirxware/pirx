//! Hardware model errors.

use thiserror::Error;

/// Errors from hardware model loading or validation.
#[derive(Debug, Error)]
pub enum HardwareModelError {
    #[error("TOML parsing failed: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("code distance must be odd and >= 3, got {0}")]
    InvalidCodeDistance(u32),

    #[error("physical error rate must be in (0, 1), got {0}")]
    InvalidPhysicalErrorRate(f64),

    #[error("error correction threshold must be in (0, 1), got {0}")]
    InvalidErrorCorrectionThreshold(f64),

    #[error("logical error prefactor must be > 0, got {0}")]
    InvalidLogicalErrorPrefactor(f64),

    #[error("cycle time must be > 0, got {0} µs")]
    InvalidCycleTime(f64),

    #[error("measurement time must be > 0, got {0} µs")]
    InvalidMeasurementTime(f64),

    #[error("classical feedback latency must be >= 0, got {0} µs")]
    InvalidFeedbackLatency(f64),

    #[error("factory count must be > 0")]
    ZeroFactories,

    #[error("buffer capacity must be > 0")]
    ZeroBufferCapacity,

    #[error("lambda_raw must be > 0 for cultivation factory, got {0}")]
    InvalidLambdaRaw(f64),

    #[error("abort probability must be in [0, 1], got {0}")]
    InvalidAbortProbability(f64),

    #[error("injection error probability must be in [0, 1], got {0}")]
    InvalidInjectionProbability(f64),

    #[error("routing overhead fraction must be in [0, 1], got {0}")]
    InvalidOverheadFraction(f64),

    #[error("factory count {0} exceeds u16::MAX (65535)")]
    FactoryCountExceedsLimit(u32),

    #[error("buffer preload {preload} exceeds capacity {capacity}")]
    PreloadExceedsCapacity { preload: u32, capacity: u32 },

    #[error("mean_cycles_per_state must be positive and finite: {0}")]
    InvalidMeanCyclesPerState(f64),

    #[error("distinct_angles must be greater than zero")]
    ZeroDistinctAngles,

    #[error("grid dimensions must be greater than zero")]
    ZeroGridDimension,

    #[error("cycles_per_hop must be greater than zero")]
    ZeroCyclesPerHop,
}
