//! Core Profiler IR types.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Opaque operation identifier within a [`ProfilerCircuit`].
pub type OpId = u64;

/// Logical qubit identifier.
pub type QubitId = u32;

/// A compiled quantum circuit in Profiler IR form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilerCircuit {
    pub ops: Vec<Operation>,
    pub deps: Vec<Dependency>,
    pub qubit_count: u32,
    pub metadata: CircuitMetadata,
}

/// A single quantum operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: OpId,
    pub kind: OpKind,
    pub qubits: SmallVec<[QubitId; 2]>,
}

/// Classification of a quantum operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// Clifford gate (H, S, CNOT, etc.) — no magic state consumed.
    Clifford,
    /// T-gate — consumes one magic state, subject to injection error.
    TGate,
    /// Pauli measurement.
    Measurement,
    /// Arbitrary rotation Rz(θ) — consumes one rotation state.
    Rotation { angle_index: u16 },
}

/// Data dependency: `from` must complete before `to` can start.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Dependency {
    pub from: OpId,
    pub to: OpId,
}

/// Circuit-level metadata computed by adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitMetadata {
    pub name: String,
    pub source_framework: String,
    pub t_count: u64,
    pub clifford_count: u64,
    pub rotation_count: u64,
    pub depth: u64,
}
