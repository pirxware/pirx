//! DAG construction from a validated circuit.

use std::collections::HashMap;

use pirx_hw::model::HardwareModel;
use pirx_ir::{
    ValidatedCircuit,
    circuit::{OpId, OpKind as IrOpKind},
};
use slotmap::{SecondaryMap, SlotMap};
use smallvec::SmallVec;
use thiserror::Error;

use super::{
    Dag,
    adjacency::DagAdjacency,
    kind::{OpData, OpKey, OpKind},
};

/// Result of [`Dag::from_circuit`]: the DAG plus the IR-to-arena key map.
///
/// The engine needs `id_to_key` to resolve measurement hook targets.
pub struct DagBuild {
    pub dag: Dag,
    pub id_to_key: HashMap<OpId, OpKey>,
}

/// Errors that can occur during DAG construction.
#[derive(Debug, Error)]
pub enum DagError {
    #[error("too many distinct rotation angles: {0} (maximum 65535)")]
    TooManyDistinctAngles(usize),

    #[error("internal error: {0}")]
    Internal(String),
}

impl Dag {
    /// Build a DAG from a [`ValidatedCircuit`].
    ///
    /// Returns [`DagBuild`] containing the DAG and a map from IR `OpId` to
    /// arena `OpKey`, needed by the engine to resolve measurement hook targets.
    ///
    /// `ValidatedCircuit` proves non-emptiness, acyclicity, unique OpIds, and
    /// valid qubit references — no structural re-validation needed here.
    ///
    /// `hw` is used only for `injection.fixup_cost_cycles`.
    pub fn from_circuit(
        circuit: &ValidatedCircuit,
        hw: &HardwareModel,
    ) -> Result<DagBuild, DagError> {
        let n = circuit.ops.len();
        let mut ops: SlotMap<OpKey, OpData> = SlotMap::with_capacity_and_key(n);
        let mut successors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>> =
            SecondaryMap::with_capacity(n);
        let mut predecessors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>> =
            SecondaryMap::with_capacity(n);
        let mut predecessor_count: SecondaryMap<OpKey, u32> = SecondaryMap::with_capacity(n);
        let mut angle_table: Vec<f64> = Vec::new();

        // Map IR OpId → arena OpKey for building adjacency.
        let mut id_to_key: HashMap<OpId, OpKey> = HashMap::with_capacity(n);

        // Measurement gates take ceil(measurement_time_us / cycle_time_us) QEC
        // cycles. All other gates default to 1 cycle; routing overhead is added
        // at scheduling time by the engine.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let measurement_cycles =
            ((hw.timing.measurement_time_us / hw.timing.cycle_time_us).ceil() as u32).max(1);

        for op in &circuit.ops {
            let kind = ir_kind_to_engine(&op.kind, &mut angle_table)?;
            let cycle_cost = if matches!(kind, OpKind::Measurement { .. }) {
                measurement_cycles
            } else {
                1
            };
            let key = ops.insert(OpData {
                kind,
                qubits: op.qubits.clone(),
                cycle_cost,
                active: op.initially_active,
            });
            successors.insert(key, SmallVec::new());
            predecessors.insert(key, SmallVec::new());
            predecessor_count.insert(key, 0);
            id_to_key.insert(op.id, key);
        }

        // Build adjacency from the dependency list.
        // ValidatedCircuit guarantees all dep endpoints exist in id_to_key.
        for dep in &circuit.deps {
            let &from_key = id_to_key.get(&dep.from).ok_or_else(|| {
                DagError::Internal(format!("dep.from {} not in id_to_key", dep.from))
            })?;
            let &to_key = id_to_key
                .get(&dep.to)
                .ok_or_else(|| DagError::Internal(format!("dep.to {} not in id_to_key", dep.to)))?;
            if let Some(succs) = successors.get_mut(from_key) {
                succs.push(to_key);
            }
            if let Some(preds) = predecessors.get_mut(to_key) {
                preds.push(from_key);
            }
            if let Some(count) = predecessor_count.get_mut(to_key) {
                *count = count.saturating_add(1);
            }
        }

        Ok(DagBuild {
            dag: Dag {
                ops,
                adjacency: DagAdjacency {
                    successors,
                    predecessors,
                    predecessor_count,
                },
                angle_table,
                fixup_cost_cycles: hw.injection.fixup_cost_cycles,
            },
            id_to_key,
        })
    }
}

/// Convert an IR `OpKind` to the engine's `OpKind`, deduplicating rotation angles.
///
/// Rotation angles are matched by bit pattern to avoid float-equality issues.
/// Measurement hook IDs are carried through for the engine to resolve at
/// dispatch time.
fn ir_kind_to_engine(kind: &IrOpKind, angle_table: &mut Vec<f64>) -> Result<OpKind, DagError> {
    match kind {
        IrOpKind::Clifford => Ok(OpKind::Clifford),
        IrOpKind::TGate => Ok(OpKind::TGate),
        IrOpKind::Measurement { hook } => Ok(OpKind::Measurement { hook: *hook }),
        IrOpKind::Rotation { angle } => {
            let bits = angle.to_bits();
            let idx = angle_table
                .iter()
                .position(|&a| a.to_bits() == bits)
                .unwrap_or_else(|| {
                    let i = angle_table.len();
                    angle_table.push(*angle);
                    i
                });
            let angle_index =
                u16::try_from(idx).map_err(|_| DagError::TooManyDistinctAngles(idx))?;
            Ok(OpKind::Rotation { angle_index })
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::super::fixtures::{chain_circuit, minimal_hw, validated};
    use crate::dag::Dag;

    #[test]
    fn from_circuit_simple_chain() {
        let hw = minimal_hw();
        let dag = Dag::from_circuit(&validated(chain_circuit(2)), &hw)
            .expect("valid chain")
            .dag;
        assert_eq!(dag.op_count(), 2);
        let roots = dag.initial_ready_set();
        assert_eq!(roots.len(), 1);
        let root = roots[0];
        assert_eq!(dag.adjacency.predecessor_count.get(root).copied(), Some(0));
        let succs = dag.adjacency.successors.get(root).expect("root has succs");
        assert_eq!(succs.len(), 1);
        let child = succs[0];
        assert_eq!(dag.adjacency.predecessor_count.get(child).copied(), Some(1));
    }
}
