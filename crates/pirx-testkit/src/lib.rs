//! Shared test fixtures for the pirx workspace.
//!
//! Every test — unit, integration, property, benchmark — imports fixture
//! builders from here. One definition, zero duplication.
//!
//! Each builder returns a valid, self-consistent object ready to use.
//! Tests that need a specific variation override individual fields after
//! construction (all config fields are `pub`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pirx_hw::{
    CodeType, RoutingConfig,
    model::{
        BufferConfig, DistillationProtocol, FactoryConfig, HardwareModel, InjectionConfig,
        MetaConfig, QecConfig, TimingConfig,
    },
};
use pirx_ir::{
    ValidatedCircuit,
    circuit::{
        CircuitMetadata, ConditionalActivation, Dependency, MeasurementHook, MeasurementOutcome,
        OpKind, Operation, ProfilerCircuit,
    },
};
use smallvec::smallvec;

/// Validate a test fixture circuit, panicking if it is invalid.
///
/// Every test fixture in this crate produces a valid circuit by construction.
/// This wrapper makes that assumption explicit and provides the
/// [`ValidatedCircuit`] proof token required by `Engine::new` and `Dag::from_circuit`.
pub fn validated(circuit: ProfilerCircuit) -> ValidatedCircuit {
    pirx_ir::validate::validate(circuit).expect("test fixture must be valid")
}

// ── Sub-config builders ──────────────────────────────────────────────────────

/// Circuit metadata with zeroed counters. Good enough for any test that
/// doesn't assert on metadata fields.
pub fn blank_meta(name: &str) -> CircuitMetadata {
    CircuitMetadata {
        name: name.into(),
        source_framework: "test".into(),
        t_count: 0,
        clifford_count: 0,
        rotation_count: 0,
        depth: 0,
    }
}

/// Surface code QEC config at the given code distance.
///
/// Physical error rate 10⁻³, threshold 10⁻², prefactor 0.038.
pub fn surface_code_qec(distance: u32) -> QecConfig {
    QecConfig {
        code_type: CodeType::SurfaceCode,
        code_distance: distance,
        physical_error_rate: 1e-3,
        error_correction_threshold: 0.01,
        logical_error_prefactor: 0.038,
    }
}

/// Standard timing: 1 µs cycle, 0.5 µs measurement, 1 µs feedback.
pub fn default_timing() -> TimingConfig {
    TimingConfig {
        cycle_time_us: 1.0,
        measurement_time_us: 0.5,
        classical_feedback_latency_us: 1.0,
    }
}

// ── Hardware model builders ──────────────────────────────────────────────────

/// Single cultivation factory, cold start.
///
/// code_distance=7, λ=0.002, injection p=0.5, fixup_cost=1,
/// buffer capacity=4, preload=0.
pub fn cultivation_hw() -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-cultivation".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
        factory: FactoryConfig::Cultivation {
            count: 1,
            lambda_raw: 0.002,
            fault_distance: 3,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::default(),
        buffer: BufferConfig {
            capacity: 4,
            preload: 0,
        },
    }
}

/// Single distillation factory (15-to-1), cold start.
///
/// 10 cycles/round × 3 rounds, abort p=0.01, code_distance=7,
/// injection p=0.5, fixup_cost=1, buffer capacity=4, preload=0.
pub fn distillation_hw() -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-distillation".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
        factory: FactoryConfig::Distillation {
            count: 1,
            protocol: DistillationProtocol::FifteenToOne,
            cycles_per_round: 10,
            rounds: 3,
            abort_probability: 0.01,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::default(),
        buffer: BufferConfig {
            capacity: 4,
            preload: 0,
        },
    }
}

/// Deterministic distillation: zero abort probability, 18 cycles/round × 3
/// rounds = exactly 54 cycles per magic state. Useful for hand-calculated
/// timing assertions.
pub fn deterministic_distillation_hw(
    factory_count: u32,
    buffer_capacity: u32,
    preload: u32,
) -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-deterministic".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
        factory: FactoryConfig::Distillation {
            count: factory_count,
            protocol: DistillationProtocol::FifteenToOne,
            cycles_per_round: 18,
            rounds: 3,
            abort_probability: 0.0,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::default(),
        buffer: BufferConfig {
            capacity: buffer_capacity,
            preload,
        },
    }
}

// ── Circuit builders ─────────────────────────────────────────────────────────

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
