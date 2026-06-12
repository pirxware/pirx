//! Hardware model specification — TOML-based parametric descriptions
//! of fault-tolerant quantum architectures.

mod config;
mod error;
pub mod model;

pub use model::{CodeType, DistillationProtocol, HardwareModel, HardwareModelError, RoutingConfig};
