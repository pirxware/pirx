//! Core Profiler IR types.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Opaque operation identifier within a [`ProfilerCircuit`].
pub type OpId = u64;

/// Logical qubit identifier.
pub type QubitId = u32;

/// Index into [`ProfilerCircuit::hooks`].
pub type MeasurementHookId = u32;

/// Logical qubit position on a 2D grid. Enables distance-aware routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPosition {
    pub qubit: QubitId,
    pub row: u32,
    pub col: u32,
}

/// Expected measurement outcome for conditional activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MeasurementOutcome {
    Zero,
    One,
}

/// Operations to activate when a measurement produces a specific outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalActivation {
    pub outcome: MeasurementOutcome,
    pub ops_to_activate: Vec<OpId>,
}

/// Pre-defined response to a measurement outcome.
///
/// Stored in [`ProfilerCircuit::hooks`], referenced by [`OpKind::Measurement`].
/// Used for algorithm-level adaptive behavior: repeat-until-success,
/// feedforward branching. Hardware-level adaptive behavior (injection
/// errors) is handled separately by the engine via `inject_fixup()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementHook {
    pub id: MeasurementHookId,
    pub activations: Vec<ConditionalActivation>,
}

/// A compiled quantum circuit in Profiler IR form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilerCircuit {
    pub ops: Vec<Operation>,
    pub deps: Vec<Dependency>,
    pub qubit_count: u32,
    /// Logical qubit positions on a 2D grid. Required for distance-aware
    /// routing models (e.g. manhattan); `None` for topology-agnostic runs.
    pub qubit_positions: Option<Vec<GridPosition>>,
    /// Algorithm-level measurement hooks. Empty for non-adaptive circuits.
    #[serde(default)]
    pub hooks: Vec<MeasurementHook>,
    pub metadata: CircuitMetadata,
}

/// A single quantum operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: OpId,
    pub kind: OpKind,
    pub qubits: SmallVec<[QubitId; 2]>,
    /// `false` for pre-allocated conditional ops (RUS iterations, branch arms).
    /// The engine skips inactive ops during ready-set computation.
    #[serde(default = "default_active")]
    pub initially_active: bool,
}

fn default_active() -> bool {
    true
}

/// Classification of a quantum operation.
///
/// `PartialEq` but not `Eq`: the `Rotation` variant holds an `f64` angle, which does not
/// implement `Eq` (NaN != NaN). Callers that need equality on non-Rotation variants may
/// match exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OpKind {
    /// Clifford gate (H, S, CNOT, etc.) — no magic state consumed.
    Clifford,
    /// T-gate — consumes one magic state, subject to injection error.
    TGate,
    /// Pauli measurement, with optional hook for conditional activation.
    Measurement { hook: Option<MeasurementHookId> },
    /// Arbitrary rotation Rz(θ) in radians — consumes one rotation state.
    Rotation { angle: f64 },
}

/// Classify an Rz rotation angle into the appropriate [`OpKind`].
///
/// - Odd multiples of π/4 → [`OpKind::TGate`]
/// - Even multiples of π/4 → [`OpKind::Clifford`]
/// - Everything else → [`OpKind::Rotation`]
#[must_use]
pub fn classify_rz_angle(angle: f64) -> OpKind {
    if !angle.is_finite() {
        return OpKind::Rotation { angle };
    }
    let k = angle / std::f64::consts::FRAC_PI_4;
    let k_rounded = k.round();
    if (k - k_rounded).abs() < 1e-10 && k_rounded.abs() < i64::MAX as f64 {
        #[allow(clippy::cast_possible_truncation)]
        let k_int = k_rounded as i64;
        if k_int % 2 != 0 {
            OpKind::TGate
        } else {
            OpKind::Clifford
        }
    } else {
        OpKind::Rotation { angle }
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use smallvec::smallvec;

    use super::*;

    /// Constructing a circuit with inactive ops and hooks compiles and
    /// all fields are accessible — validates the public API surface.
    #[test]
    fn construct_circuit_with_hooks_and_inactive_ops() {
        let circuit = ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: OpKind::Measurement { hook: Some(0) },
                    qubits: smallvec![0],
                    initially_active: true,
                },
                Operation {
                    id: 1,
                    kind: OpKind::Clifford,
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![MeasurementHook {
                id: 0,
                activations: vec![ConditionalActivation {
                    outcome: MeasurementOutcome::One,
                    ops_to_activate: vec![1],
                }],
            }],
            metadata: CircuitMetadata {
                name: "hook-test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 1,
                rotation_count: 0,
                depth: 2,
            },
        };

        assert_eq!(circuit.ops.len(), 2);
        assert!(circuit.ops[0].initially_active);
        assert!(!circuit.ops[1].initially_active);
        assert_eq!(circuit.hooks.len(), 1);
        assert_eq!(
            circuit.hooks[0].activations[0].outcome,
            MeasurementOutcome::One
        );
        assert_eq!(circuit.hooks[0].activations[0].ops_to_activate, vec![1]);
    }

    /// Grid positions are correctly stored and derive Eq for comparisons.
    #[test]
    fn grid_position_equality() {
        let a = GridPosition {
            qubit: 0,
            row: 3,
            col: 5,
        };
        let b = GridPosition {
            qubit: 0,
            row: 3,
            col: 5,
        };
        let c = GridPosition {
            qubit: 0,
            row: 3,
            col: 6,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// Measurement { hook: None } preserves Copy — adding the struct field
    /// to the variant must not break this invariant for the hot loop.
    #[test]
    fn measurement_hook_none_is_copy() {
        let kind = OpKind::Measurement { hook: None };
        let copy = kind;
        assert_eq!(kind, copy);
    }

    /// Measurement { hook: Some(id) } is also Copy (Option<u32> is Copy).
    #[test]
    fn measurement_hook_some_is_copy() {
        let kind = OpKind::Measurement { hook: Some(42) };
        let copy = kind;
        assert_eq!(kind, copy);
    }
}
