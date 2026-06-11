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
}

/// Validate that a [`ProfilerCircuit`] is well-formed.
///
/// Checks:
/// - Circuit is non-empty
/// - No duplicate operation IDs
/// - All dependency endpoints reference existing operations
/// - All qubit references are within `qubit_count`
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

    // Validate dependency references.
    for dep in &circuit.deps {
        if !id_set.contains(&dep.from) {
            return Err(ValidationError::DanglingDependency(dep.from));
        }
        if !id_set.contains(&dep.to) {
            return Err(ValidationError::DanglingDependency(dep.to));
        }
    }

    // Acyclicity check via Kahn's algorithm (topological sort).
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
                && let Some(count) = in_degree.get_mut(&dep.to) {
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
    use crate::circuit::{CircuitMetadata, Dependency, OpKind, Operation, ProfilerCircuit};
    use smallvec::smallvec;

    fn minimal_circuit() -> ProfilerCircuit {
        ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: OpKind::Clifford,
                    qubits: smallvec![0],
                },
                Operation {
                    id: 1,
                    kind: OpKind::TGate,
                    qubits: smallvec![0],
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }],
            qubit_count: 1,
            metadata: CircuitMetadata {
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 1,
                clifford_count: 1,
                rotation_count: 0,
                depth: 2,
            },
        }
    }

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
        });
        assert!(matches!(
            validate(&c),
            Err(ValidationError::DuplicateOpId(0))
        ));
    }
}
