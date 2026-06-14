//! Parameter sensitivity analysis for fault-tolerant quantum hardware models.
//!
//! Provides Morris and Sobol methods over the hardware parameter space,
//! leveraging the pirx-core simulation engine for evaluation.

pub mod error;
pub mod metric;
pub mod morris;
pub mod mutate;
pub mod parameter;
pub mod sample;
pub mod sobol;
pub mod sobol_sequence;

pub mod config;

pub use config::{MorrisConfig, SensitivityConfig, parse_sensitivity_config};
pub use error::SensitivityError;
pub use metric::OutputMetric;
pub use morris::{MorrisParameterResult, MorrisResult, morris_screening};
pub use mutate::{mutate_hw, mutate_hw_multi};
pub use parameter::{ParameterDef, ParameterKind, ParameterSpace};
pub use sample::{EvalConfig, evaluate_point};
