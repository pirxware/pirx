//! Validation of Profiler IR circuits.

use std::collections::VecDeque;

use crate::circuit::ProfilerCircuit;

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
pub fn validate(circuit: &ProfilerCircuit) -> Result<(), ValidationError> {
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

    // Validate hook activation targets: must exist and be inactive.
    for hook in &circuit.hooks {
        for activation in &hook.activations {
            for &op_id in &activation.ops_to_activate {
                let Some(op) = circuit.ops.iter().find(|o| o.id == op_id) else {
                    return Err(ValidationError::DanglingDependency(op_id));
                };
                if op.initially_active {
                    return Err(ValidationError::ActiveOpInHook(op_id));
                }
            }
        }
    }

    // No orphaned inactive ops: every inactive op must be reachable from a hook.
    for op in &circuit.ops {
        if !op.initially_active {
            let referenced = circuit.hooks.iter().any(|h| {
                h.activations
                    .iter()
                    .any(|a| a.ops_to_activate.contains(&op.id))
            });
            if !referenced {
                return Err(ValidationError::OrphanedInactiveOp(op.id));
            }
        }
    }

    // Acyclicity check via Kahn's algorithm (topological sort).
    // Runs on ALL ops (including inactive) — the full DAG must be acyclic.
    let mut in_degree = std::collections::HashMap::with_capacity(circuit.ops.len());
    for op in &circuit.ops {
        in_degree.insert(op.id, 0u64);
    }
    for dep in &circuit.deps {
        *in_degree.entry(dep.to).or_insert(0) += 1;
    }

    let mut queue = VecDeque::new();
    for (&id, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(id);
        }
    }

    let mut visited = 0u64;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for dep in &circuit.deps {
            if dep.from == node
                && let Some(count) = in_degree.get_mut(&dep.to)
            {
                *count -= 1;
                if *count == 0 {
                    queue.push_back(dep.to);
                }
            }
        }
    }

    if visited != circuit.ops.len() as u64 {
        return Err(ValidationError::CyclicDag);
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::circuit::{
        CircuitMetadata, ConditionalActivation, Dependency, GridPosition, MeasurementHook,
        MeasurementOutcome, OpKind, Operation, ProfilerCircuit,
    };
    use smallvec::smallvec;

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
        assert!(validate(&minimal_circuit()).is_ok());
    }

    #[test]
    fn empty_circuit_rejected() {
        let mut c = minimal_circuit();
        c.ops.clear();
        assert!(matches!(validate(&c), Err(ValidationError::EmptyCircuit)));
    }

    #[test]
    fn dangling_dependency_rejected() {
        let mut c = minimal_circuit();
        c.deps.push(Dependency { from: 0, to: 99 });
        assert!(matches!(
            validate(&c),
            Err(ValidationError::DanglingDependency(99))
        ));
    }

    #[test]
    fn invalid_qubit_rejected() {
        let mut c = minimal_circuit();
        c.ops[0].qubits = smallvec![5];
        assert!(matches!(
            validate(&c),
            Err(ValidationError::InvalidQubit { .. })
        ));
    }

    #[test]
    fn cyclic_dag_rejected() {
        let mut c = minimal_circuit();
        c.deps.push(Dependency { from: 1, to: 0 });
        assert!(matches!(validate(&c), Err(ValidationError::CyclicDag)));
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
            validate(&c),
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
        assert!(validate(&c).is_ok());
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
            validate(&c),
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
            validate(&c),
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
            validate(&c),
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
        assert!(validate(&c).is_ok());
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
            validate(&c),
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
            validate(&c),
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
            validate(&c),
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
            validate(&c),
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
        assert!(validate(&c).is_ok());
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
        assert!(validate(&c).is_ok());
    }
}
