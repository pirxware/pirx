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

pub use error::SensitivityError;
pub use mutate::{mutate_hw, mutate_hw_multi};
pub use parameter::{KNOWN_PARAMS, ParameterDef, ParameterKind, ParameterSpace};
