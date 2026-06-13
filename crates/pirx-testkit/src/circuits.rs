//! Shared circuit fixtures for the pirx workspace.

use pirx_ir::circuit::{
    CircuitMetadata, ConditionalActivation, Dependency, GridPosition, MeasurementHook,
    MeasurementOutcome, OpKind, Operation, ProfilerCircuit,
};
use smallvec::smallvec;

use crate::hardware::blank_meta;

/// `n` independent two-qubit Clifford gates on a grid. Qubit `2*i` and
/// `2*i+1` form each pair, laid out on consecutive grid columns.
/// Used for benchmarking Manhattan routing with many qubits.
pub fn two_qubit_grid(n: u32) -> ProfilerCircuit {
    let qubit_count = n * 2;
    let ops: Vec<Operation> = (0..n)
        .map(|i| Operation {
            id: u64::from(i),
            kind: OpKind::Clifford,
            qubits: smallvec![i * 2, i * 2 + 1],
            initially_active: true,
        })
        .collect();
    let positions: Vec<GridPosition> = (0..qubit_count)
        .map(|q| GridPosition {
            qubit: q,
            row: q / 64,
            col: q % 64,
        })
        .collect();
    ProfilerCircuit {
        ops,
        deps: vec![],
        qubit_count,
        qubit_positions: Some(positions),
        hooks: vec![],
        metadata: CircuitMetadata {
            name: "two-qubit-grid".into(),
            source_framework: "test".into(),
            t_count: 0,
            clifford_count: u64::from(n),
            rotation_count: 0,
            depth: 1,
        },
    }
}

/// Single Clifford gate on qubit 0. No dependencies, no magic state needed.
pub fn single_clifford() -> ProfilerCircuit {
    ProfilerCircuit {
        ops: vec![Operation {
            id: 0,
            kind: OpKind::Clifford,
            qubits: smallvec![0],
            initially_active: true,
        }],
        deps: vec![],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: CircuitMetadata {
            name: "single-clifford".into(),
            source_framework: "test".into(),
            t_count: 0,
            clifford_count: 1,
            rotation_count: 0,
            depth: 1,
        },
    }
}

/// Single T-gate on qubit 0. Requires one magic state.
pub fn single_t_gate() -> ProfilerCircuit {
    ProfilerCircuit {
        ops: vec![Operation {
            id: 0,
            kind: OpKind::TGate,
            qubits: smallvec![0],
            initially_active: true,
        }],
        deps: vec![],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("single-t-gate"),
    }
}

/// Two independent T-gates on separate qubits. Both enter the ready set
/// simultaneously — tests parallel magic state consumption.
pub fn two_parallel_t_gates() -> ProfilerCircuit {
    ProfilerCircuit {
        ops: vec![
            Operation {
                id: 0,
                kind: OpKind::TGate,
                qubits: smallvec![0],
                initially_active: true,
            },
            Operation {
                id: 1,
                kind: OpKind::TGate,
                qubits: smallvec![1],
                initially_active: true,
            },
        ],
        deps: vec![],
        qubit_count: 2,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("two-parallel-t-gates"),
    }
}

/// Linear chain of `n` Clifford gates on qubit 0: op(0) → op(1) → … → op(n-1).
pub fn clifford_chain(n: u32) -> ProfilerCircuit {
    let ops = (0..n)
        .map(|i| Operation {
            id: u64::from(i),
            kind: OpKind::Clifford,
            qubits: smallvec![0],
            initially_active: true,
        })
        .collect();
    let deps = (0..n.saturating_sub(1))
        .map(|i| Dependency {
            from: u64::from(i),
            to: u64::from(i + 1),
        })
        .collect();
    ProfilerCircuit {
        ops,
        deps,
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("clifford-chain"),
    }
}

/// Linear chain of `n` T-gates on qubit 0. Each gate consumes one magic
/// state and may trigger an injection error with fixup.
pub fn t_gate_chain(n: u32) -> ProfilerCircuit {
    let ops: Vec<Operation> = (0..n)
        .map(|i| Operation {
            id: u64::from(i),
            kind: OpKind::TGate,
            qubits: smallvec![0],
            initially_active: true,
        })
        .collect();
    let deps: Vec<Dependency> = (0..n.saturating_sub(1))
        .map(|i| Dependency {
            from: u64::from(i),
            to: u64::from(i + 1),
        })
        .collect();
    ProfilerCircuit {
        ops,
        deps,
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: CircuitMetadata {
            name: "t-gate-chain".into(),
            source_framework: "test".into(),
            t_count: u64::from(n),
            clifford_count: 0,
            rotation_count: 0,
            depth: u64::from(n),
        },
    }
}

/// Clifford(0) → TGate(1) → Measurement(2): exercises the full gate lifecycle
/// including dependency ordering, magic state consumption, and injection errors.
pub fn clifford_t_measurement_chain() -> ProfilerCircuit {
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
            Operation {
                id: 2,
                kind: OpKind::Measurement { hook: None },
                qubits: smallvec![0],
                initially_active: true,
            },
        ],
        deps: vec![Dependency { from: 0, to: 1 }, Dependency { from: 1, to: 2 }],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("clifford-t-measurement"),
    }
}

/// Measurement(0) → hook → Clifford(1, inactive).
///
/// Outcome `One` activates op 1. Only one outcome branch, so for seeds that
/// produce `Zero`, op 1 stays inactive and the circuit completes with 1 op.
pub fn measurement_with_one_hook() -> ProfilerCircuit {
    ProfilerCircuit {
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
        metadata: blank_meta("hook-one-branch"),
    }
}

/// Measurement(0) → hook → Clifford(1, Zero branch) or Clifford(2, One branch).
///
/// Both outcomes activate exactly one op. Verifies that exactly one branch
/// executes per run and the other stays inactive.
pub fn measurement_with_both_outcomes() -> ProfilerCircuit {
    ProfilerCircuit {
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
        metadata: blank_meta("hook-both-branches"),
    }
}

/// Measurement(0) → hook → TGate(1, One branch).
///
/// The T-gate activated by hook can trigger injection error + fixup insertion.
/// Tests hook + injection error interaction.
pub fn hook_activates_t_gate() -> ProfilerCircuit {
    ProfilerCircuit {
        ops: vec![
            Operation {
                id: 0,
                kind: OpKind::Measurement { hook: Some(0) },
                qubits: smallvec![0],
                initially_active: true,
            },
            Operation {
                id: 1,
                kind: OpKind::TGate,
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
        metadata: blank_meta("hook-activates-t"),
    }
}

/// Single Rotation gate on qubit 0. Requires one magic state (rotation state).
pub fn single_rotation() -> ProfilerCircuit {
    ProfilerCircuit {
        ops: vec![Operation {
            id: 0,
            kind: OpKind::Rotation {
                angle: 0.123_456_789,
            },
            qubits: smallvec![0],
            initially_active: true,
        }],
        deps: vec![],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: CircuitMetadata {
            name: "single-rotation".into(),
            source_framework: "test".into(),
            t_count: 0,
            clifford_count: 0,
            rotation_count: 1,
            depth: 1,
        },
    }
}

/// `n` independent Clifford gates, each on a separate qubit. All enter the
/// ready set at once — tests parallel scheduling.
pub fn parallel_cliffords(n: u32) -> ProfilerCircuit {
    let ops = (0..n)
        .map(|i| Operation {
            id: u64::from(i),
            kind: OpKind::Clifford,
            qubits: smallvec![i],
            initially_active: true,
        })
        .collect();
    ProfilerCircuit {
        ops,
        deps: vec![],
        qubit_count: n,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("parallel-cliffords"),
    }
}
