//! Shared test fixtures for the pirx workspace.
//!
//! Every test — unit, integration, property, benchmark — imports fixture
//! builders from here. One definition, zero duplication.
//!
//! Each builder returns a valid, self-consistent object ready to use.
//! Tests that need a specific variation override individual fields after
//! construction (all config fields are `pub`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod circuits;
pub mod hardware;

// Re-export everything for backward compatibility.
pub use circuits::*;
pub use hardware::*;
use pirx_ir::{ValidatedCircuit, circuit::ProfilerCircuit};

/// Validate a test fixture circuit, panicking if it is invalid.
///
/// Every test fixture in this crate produces a valid circuit by construction.
/// This wrapper makes that assumption explicit and provides the
/// [`ValidatedCircuit`] proof token required by `Engine::new` and `Dag::from_circuit`.
pub fn validated(circuit: ProfilerCircuit) -> ValidatedCircuit {
    pirx_ir::validate::validate(circuit).expect("test fixture must be valid")
}
