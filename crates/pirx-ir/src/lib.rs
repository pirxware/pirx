//! Profiler IR — framework-agnostic circuit representation.
//!
//! Defines the minimal data structures the DES engine needs:
//! operations, types, qubit assignments, and data dependencies.
//! This crate never references any external quantum framework.

pub mod circuit;
pub mod validate;

pub use validate::ValidatedCircuit;
