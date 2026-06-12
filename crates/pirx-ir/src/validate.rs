//! Validation of Profiler IR circuits.

use std::collections::VecDeque;

use crate::circuit::ProfilerCircuit;

/// A [`ProfilerCircuit`] that has passed all validation checks.
///
/// Created only by [`validate()`]. Cannot be constructed directly.
/// `Deref` provides transparent read access to the inner circuit.
///
/// Implements `Serialize` via the inner circuit (for `.pirx.json` interchange).
/// Does **not** implement `Deserialize` — deserialized circuits must go through
/// [`validate()`] to obtain a new proof token.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub struct ValidatedCircuit {
    inner: ProfilerCircuit,
}

impl std::ops::Deref for ValidatedCircuit {
    type Target = ProfilerCircuit;
    fn deref(&self) -> &ProfilerCircuit {
        &self.inner
    }
}

/// Errors detected during IR validation.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Circuit contains no operations")]
    EmptyCircuit,

    #[error("Dependency references non-existent operation ID {0}")]
    DanglingDependency(u64),

    #[error("Operation {op_id} references qubit {qubit_id} beyond qubit_count {qubit_count}")]
    InvalidQubit {
        op_id: u64,
        qubit_id: u32,
        qubit_count: u32,
    },

    #[error("Dependency graph contains a cycle")]
    CyclicDag,

    #[error("Duplicate operation ID {0}")]
    DuplicateOpId(u64),

    #[error("qubit_positions length {got} does not match qubit_count {expected}")]
    QubitPositionCountMismatch { expected: u32, got: u32 },

    #[error("duplicate qubit position for qubit {0}")]
    DuplicateQubitPosition(u32),

    #[error("qubit position references qubit {qubit_id} beyond qubit_count {qubit_count}")]
    QubitPositionInvalidQubit { qubit_id: u32, qubit_count: u32 },

    #[error("measurement references non-existent hook {0}")]
    DanglingHookReference(u32),

    #[error("hook references op {0} which has initially_active=true (must be false)")]
    ActiveOpInHook(u64),

    #[error("op {0} has initially_active=false but is not referenced by any hook")]
    OrphanedInactiveOp(u64),
}

/// Validate that a [`ProfilerCircuit`] is well-formed.
///
/// Checks:
/// - Circuit is non-empty
/// - No duplicate operation IDs
/// - All qubit references are within `qubit_count`
/// - Qubit positions (if present) match `qubit_count`, are unique, and in range
/// - All dependency endpoints reference existing operations
/// - Measurement hook references resolve to existing hooks
/// - Hook activation targets reference existing, inactive operations
/// - No orphaned inactive ops (every inactive op reachable from a hook)
/// - Dependency graph is acyclic (topological sort via Kahn's algorithm)
pub fn validate(circuit: ProfilerCircuit) -> Result<ValidatedCircuit, ValidationError> {
    if circuit.ops.is_empty() {
        return Err(ValidationError::EmptyCircuit);
    }

    // Build an ID set for O(1) existence checks.
    let mut id_set = std::collections::HashSet::with_capacity(circuit.ops.len());
    for op in &circuit.ops {
        if !id_set.insert(op.id) {
            return Err(ValidationError::DuplicateOpId(op.id));
        }
    }

    // Validate qubit references.
    for op in &circuit.ops {
        for &q in &op.qubits {
            if q >= circuit.qubit_count {
                return Err(ValidationError::InvalidQubit {
                    op_id: op.id,
                    qubit_id: q,
                    qubit_count: circuit.qubit_count,
                });
            }
        }
    }

    // Validate qubit positions (if present).
    if let Some(positions) = &circuit.qubit_positions {
        if positions.len() != circuit.qubit_count as usize {
            return Err(ValidationError::QubitPositionCountMismatch {
                expected: circuit.qubit_count,
                got: u32::try_from(positions.len()).unwrap_or(u32::MAX),
            });
        }
        let mut seen = std::collections::HashSet::new();
        for pos in positions {
            if pos.qubit >= circuit.qubit_count {
                return Err(ValidationError::QubitPositionInvalidQubit {
                    qubit_id: pos.qubit,
                    qubit_count: circuit.qubit_count,
                });
            }
            if !seen.insert(pos.qubit) {
                return Err(ValidationError::DuplicateQubitPosition(pos.qubit));
            }
        }
    }

    // Validate dependency references.
    for dep in &circuit.deps {
        if !id_set.contains(&dep.from) {
            return Err(ValidationError::DanglingDependency(dep.from));
        }
        if !id_set.contains(&dep.to) {
            return Err(ValidationError::DanglingDependency(dep.to));
        }
    }

    // Validate measurement hook references.
    for op in &circuit.ops {
        if let crate::circuit::OpKind::Measurement { hook: Some(id) } = op.kind
            && circuit.hooks.iter().all(|h| h.id != id)
        {
            return Err(ValidationError::DanglingHookReference(id));
        }
    }

    // Build id → index map for O(1) op lookups in hook validation and Kahn's.
    let id_to_idx: std::collections::HashMap<u64, usize> = circuit
        .ops
        .iter()
        .enumerate()
        .map(|(i, op)| (op.id, i))
        .collect();

    // Validate hook activation targets: must exist and be inactive.
    for hook in &circuit.hooks {
        for activation in &hook.activations {
            for &op_id in &activation.ops_to_activate {
                let Some(&idx) = id_to_idx.get(&op_id) else {
                    return Err(ValidationError::DanglingDependency(op_id));
                };
                if let Some(op) = circuit.ops.get(idx)
                    && op.initially_active
                {
                    return Err(ValidationError::ActiveOpInHook(op_id));
                }
            }
        }
    }

    // No orphaned inactive ops: every inactive op must be reachable from a hook.
    let hook_targets: std::collections::HashSet<u64> = circuit
        .hooks
        .iter()
        .flat_map(|h| {
            h.activations
                .iter()
                .flat_map(|a| a.ops_to_activate.iter().copied())
        })
        .collect();
    for op in &circuit.ops {
        if !op.initially_active && !hook_targets.contains(&op.id) {
            return Err(ValidationError::OrphanedInactiveOp(op.id));
        }
    }

    // Acyclicity check via Kahn's algorithm on index-based adjacency — O(V+E).
    let num_ops = circuit.ops.len();
    let mut in_degree = vec![0u32; num_ops];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); num_ops];
    for dep in &circuit.deps {
        // Both endpoints already validated against id_set above.
        if let (Some(&from_idx), Some(&to_idx)) = (id_to_idx.get(&dep.from), id_to_idx.get(&dep.to))
        {
            if let Some(succs) = successors.get_mut(from_idx) {
                succs.push(to_idx);
            }
            if let Some(deg) = in_degree.get_mut(to_idx) {
                *deg = deg.saturating_add(1);
            }
        }
    }

    let mut queue = VecDeque::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(idx);
        }
    }

    let mut visited = 0u64;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        let succs = successors.get(node).map_or(&[] as &[usize], Vec::as_slice);
        for &succ in succs {
            if let Some(deg) = in_degree.get_mut(succ) {
                *deg = deg.saturating_sub(1);
                if *deg == 0 {
                    queue.push_back(succ);
                }
            }
        }
    }

    if visited != num_ops as u64 {
        return Err(ValidationError::CyclicDag);
    }

    Ok(ValidatedCircuit { inner: circuit })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use smallvec::smallvec;

    use super::*;
    use crate::circuit::{
        CircuitMetadata, ConditionalActivation, Dependency, GridPosition, MeasurementHook,
        MeasurementOutcome, OpKind, Operation, ProfilerCircuit,
    };

    fn test_meta() -> CircuitMetadata {
        CircuitMetadata {
            name: "test".into(),
            source_framework: "test".into(),
            t_count: 1,
            clifford_count: 1,
            rotation_count: 0,
            depth: 2,
        }
    }

    fn minimal_circuit() -> ProfilerCircuit {
        ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: OpKind::Clifford,
                    qubits: smallvec![0],
                    initially_active: true,
                },
                Operation {
                    id: 1,
                    kind: OpKind::TGate,
                    qubits: smallvec![0],
                    initially_active: true,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: test_meta(),
        }
    }

    // ── Existing validations ────────────────────────────────────────────────

    #[test]
    fn valid_circuit_passes() {
        assert!(validate(minimal_circuit()).is_ok());
    }

    #[test]
    fn empty_circuit_rejected() {
        let mut c = minimal_circuit();
        c.ops.clear();
        assert!(matches!(validate(c), Err(ValidationError::EmptyCircuit)));
    }

    #[test]
    fn dangling_dependency_rejected() {
        let mut c = minimal_circuit();
        c.deps.push(Dependency { from: 0, to: 99 });
        assert!(matches!(
            validate(c),
            Err(ValidationError::DanglingDependency(99))
        ));
    }

    #[test]
    fn invalid_qubit_rejected() {
        let mut c = minimal_circuit();
        c.ops[0].qubits = smallvec![5];
        assert!(matches!(
            validate(c),
            Err(ValidationError::InvalidQubit { .. })
        ));
    }

    #[test]
    fn cyclic_dag_rejected() {
        let mut c = minimal_circuit();
        c.deps.push(Dependency { from: 1, to: 0 });
        assert!(matches!(validate(c), Err(ValidationError::CyclicDag)));
    }

    #[test]
    fn duplicate_op_id_rejected() {
        let mut c = minimal_circuit();
        c.ops.push(Operation {
            id: 0,
            kind: OpKind::Clifford,
            qubits: smallvec![0],
            initially_active: true,
        });
        assert!(matches!(
            validate(c),
            Err(ValidationError::DuplicateOpId(0))
        ));
    }

    // ── Qubit positions (Patch 1) ───────────────────────────────────────────

    #[test]
    fn valid_positions_passes() {
        let mut c = minimal_circuit();
        c.qubit_positions = Some(vec![GridPosition {
            qubit: 0,
            row: 0,
            col: 0,
        }]);
        assert!(validate(c).is_ok());
    }

    #[test]
    fn position_count_mismatch_rejected() {
        let mut c = minimal_circuit();
        c.qubit_positions = Some(vec![
            GridPosition {
                qubit: 0,
                row: 0,
                col: 0,
            },
            GridPosition {
                qubit: 1,
                row: 0,
                col: 1,
            },
        ]);
        assert!(matches!(
            validate(c),
            Err(ValidationError::QubitPositionCountMismatch {
                expected: 1,
                got: 2
            })
        ));
    }

    #[test]
    fn duplicate_qubit_position_rejected() {
        let mut c = minimal_circuit();
        c.qubit_count = 2;
        c.qubit_positions = Some(vec![
            GridPosition {
                qubit: 0,
                row: 0,
                col: 0,
            },
            GridPosition {
                qubit: 0,
                row: 1,
                col: 0,
            },
        ]);
        assert!(matches!(
            validate(c),
            Err(ValidationError::DuplicateQubitPosition(0))
        ));
    }

    #[test]
    fn position_qubit_out_of_range_rejected() {
        let mut c = minimal_circuit();
        c.qubit_positions = Some(vec![GridPosition {
            qubit: 99,
            row: 0,
            col: 0,
        }]);
        assert!(matches!(
            validate(c),
            Err(ValidationError::QubitPositionInvalidQubit {
                qubit_id: 99,
                qubit_count: 1
            })
        ));
    }

    // ── Measurement hooks and inactive ops (Patches 2-3) ────────────────────

    #[test]
    fn valid_hook_passes() {
        let c = ProfilerCircuit {
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
            metadata: test_meta(),
        };
        assert!(validate(c).is_ok());
    }

    #[test]
    fn dangling_hook_reference_rejected() {
        let c = ProfilerCircuit {
            ops: vec![Operation {
                id: 0,
                kind: OpKind::Measurement { hook: Some(99) },
                qubits: smallvec![0],
                initially_active: true,
            }],
            deps: vec![],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: test_meta(),
        };
        assert!(matches!(
            validate(c),
            Err(ValidationError::DanglingHookReference(99))
        ));
    }

    #[test]
    fn hook_references_nonexistent_op_rejected() {
        let c = ProfilerCircuit {
            ops: vec![Operation {
                id: 0,
                kind: OpKind::Measurement { hook: Some(0) },
                qubits: smallvec![0],
                initially_active: true,
            }],
            deps: vec![],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![MeasurementHook {
                id: 0,
                activations: vec![ConditionalActivation {
                    outcome: MeasurementOutcome::One,
                    ops_to_activate: vec![999],
                }],
            }],
            metadata: test_meta(),
        };
        assert!(matches!(
            validate(c),
            Err(ValidationError::DanglingDependency(999))
        ));
    }

    #[test]
    fn hook_references_active_op_rejected() {
        let c = ProfilerCircuit {
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
                    initially_active: true, // must be false for hook target
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
            metadata: test_meta(),
        };
        assert!(matches!(
            validate(c),
            Err(ValidationError::ActiveOpInHook(1))
        ));
    }

    #[test]
    fn orphaned_inactive_op_rejected() {
        let c = ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: OpKind::Clifford,
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
            hooks: vec![],
            metadata: test_meta(),
        };
        assert!(matches!(
            validate(c),
            Err(ValidationError::OrphanedInactiveOp(1))
        ));
    }

    #[test]
    fn measurement_without_hook_passes() {
        let c = ProfilerCircuit {
            ops: vec![Operation {
                id: 0,
                kind: OpKind::Measurement { hook: None },
                qubits: smallvec![0],
                initially_active: true,
            }],
            deps: vec![],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: test_meta(),
        };
        assert!(validate(c).is_ok());
    }

    #[test]
    fn multiple_hooks_both_outcomes_passes() {
        let c = ProfilerCircuit {
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
                Operation {
                    id: 2,
                    kind: OpKind::Clifford,
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }, Dependency { from: 0, to: 2 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![MeasurementHook {
                id: 0,
                activations: vec![
                    ConditionalActivation {
                        outcome: MeasurementOutcome::Zero,
                        ops_to_activate: vec![1],
                    },
                    ConditionalActivation {
                        outcome: MeasurementOutcome::One,
                        ops_to_activate: vec![2],
                    },
                ],
            }],
            metadata: test_meta(),
        };
        assert!(validate(c).is_ok());
    }
}
