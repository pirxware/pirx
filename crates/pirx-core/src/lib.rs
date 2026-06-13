//! Pirx core — discrete-event simulation engine for FTQC execution profiling.
//!
//! This crate contains the DES engine, factory models, injection error
//! model, trace collection, and post-hoc analysis. It depends on
//! [`pirx_ir`] for the circuit representation and [`pirx_hw`] for the
//! hardware model — nothing else.

pub mod analysis;
pub(crate) mod buffer;
pub mod dag;
pub mod engine;
pub(crate) mod events;
pub mod factory;
pub mod monte_carlo;
pub(crate) mod routing;
pub mod trace;

// Re-export primary types for downstream ergonomics.
pub use analysis::{BottleneckType, ExecutionProfile, ProfileAnalyzer, StallRecord};
pub use engine::{Engine, EngineConfig, EngineError};
pub use monte_carlo::{
    Distribution, MonteCarloConfig, MonteCarloError, MonteCarloResult, ReplicaSummary,
    run_monte_carlo,
};
pub use trace::{SYNTHETIC_ID_FLAG, Trace, TraceEvent, TraceEventKind};
