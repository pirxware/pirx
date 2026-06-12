//! Operation types and node data for the circuit DAG.

use pirx_ir::circuit::{MeasurementHookId, QubitId};
use serde::{Deserialize, Serialize};
use slotmap::new_key_type;
use smallvec::SmallVec;

new_key_type! {
    /// Arena key for a single operation in the DAG.
    ///
    /// Generational — stale keys are detected after injection-error insertions,
    /// preventing ABA bugs.
    pub struct OpKey;
}

/// Engine-internal operation classification.
///
/// Separate from [`pirx_ir::circuit::OpKind`]: uses `angle_index: u16` instead
/// of `f64` for rotations, and adds [`OpKind::Fixup`] for injection-error nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// Clifford gate — no magic state consumed, no injection error possible.
    Clifford,
    /// T-gate — consumes one magic state, subject to injection error.
    TGate,
    /// Pauli measurement, with optional hook for conditional activation.
    Measurement { hook: Option<MeasurementHookId> },
    /// Arbitrary-angle rotation mapped to a synthesis unit by angle index.
    Rotation {
        /// Index into [`Dag::angle_table`]. Deduplicated during [`Dag::from_circuit`].
        angle_index: u16,
    },
    /// Fixup node inserted after a T-gate injection error. Immediately ready.
    Fixup,
}

/// Core node data — hot during gate scheduling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpData {
    pub kind: OpKind,
    /// Logical qubits this operation acts on (1-2 in the common case).
    pub qubits: SmallVec<[QubitId; 2]>,
    /// Execution cost in QEC cycles.
    pub cycle_cost: u32,
    /// Inactive ops are skipped in ready-set computation.
    /// Activated by measurement hooks during simulation.
    pub active: bool,
}
