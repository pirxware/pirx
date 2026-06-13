//! Circuit dependency graph for the DES engine.
//!
//! `Dag` holds the operation arena, adjacency maps, and rotation-angle dedup
//! table. Simulation state (current cycle, event queue, stalled gates) lives
//! in the Engine, not here. Mutation is exposed only via
//! [`Dag::release_successors`], [`Dag::inject_fixup`], and [`Dag::activate_ops`].

mod adjacency;
mod build;
mod kind;
mod mutate;
mod ready_queue;

pub use adjacency::DagAdjacency;
pub use build::{DagBuild, DagError};
pub use kind::{OpData, OpKey, OpKind};
pub use ready_queue::FifoReadyQueue;
use slotmap::SlotMap;

/// Circuit dependency graph — the engine's structural view of the circuit.
///
/// Owns the operation arena, adjacency maps, and the rotation-angle dedup
/// table. The Engine drives simulation; the DAG provides operations, edges,
/// and the mutation primitives used during the simulation loop.
///
/// Callers should prefer [`Dag::release_successors`] and [`Dag::inject_fixup`]
/// over direct mutation of `adjacency`.
#[derive(Debug)]
pub struct Dag {
    pub(crate) ops: SlotMap<OpKey, OpData>,
    /// Adjacency maps — `pub` so the Engine can read them directly for
    /// trace-event generation without a method-call overhead.
    pub adjacency: DagAdjacency,
    /// Deduped rotation angles. `OpKind::Rotation { angle_index }` indexes here.
    pub(crate) angle_table: Vec<f64>,
    /// QEC cycles per injected fixup node (from `HardwareModel::injection`).
    pub(crate) fixup_cost_cycles: u32,
}

impl Dag {
    /// Return all operations with no predecessors — the initial ready set.
    ///
    /// Call once after construction to seed the engine's ready queue. After
    /// `release_successors` is called during simulation, predecessor counts
    /// change; the result of subsequent calls reflects current (not initial) state.
    pub fn initial_ready_set(&self) -> Vec<OpKey> {
        self.ops
            .keys()
            .filter(|&k| {
                self.ops.get(k).is_some_and(|op| op.active)
                    && self
                        .adjacency
                        .predecessor_count
                        .get(k)
                        .copied()
                        .unwrap_or(0)
                        == 0
            })
            .collect()
    }

    /// Get the data for an operation by key.
    pub fn get(&self, key: OpKey) -> Option<&OpData> {
        self.ops.get(key)
    }

    /// The rotation angle (in radians) for a given `angle_index`.
    pub fn angle_table(&self) -> &[f64] {
        &self.angle_table
    }

    /// Total number of operations currently in the DAG, including injected fixups.
    pub fn op_count(&self) -> usize {
        self.ops.len()
    }

    /// Number of initially active operations (excludes inactive hook targets).
    pub fn active_op_count(&self) -> usize {
        self.ops.values().filter(|op| op.active).count()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod fixtures {
    use pirx_hw::{
        CodeType, RoutingConfig,
        model::{
            BufferConfig, DistillationProtocol, FactoryConfig, HardwareModel, InjectionConfig,
            MetaConfig, QecConfig, TimingConfig,
        },
    };
    use pirx_ir::{
        ValidatedCircuit,
        circuit::{CircuitMetadata, Dependency, OpKind as IrOpKind, Operation, ProfilerCircuit},
    };
    use smallvec::smallvec;

    pub fn validated(circuit: ProfilerCircuit) -> ValidatedCircuit {
        pirx_ir::validate::validate(circuit).expect("test fixture must be valid")
    }

    pub fn meta() -> CircuitMetadata {
        CircuitMetadata {
            name: "test".into(),
            source_framework: "test".into(),
            t_count: 0,
            clifford_count: 0,
            rotation_count: 0,
            depth: 0,
        }
    }

    pub fn minimal_hw() -> HardwareModel {
        HardwareModel {
            meta: MetaConfig {
                name: "test".into(),
                description: String::new(),
            },
            qec: QecConfig {
                code_type: CodeType::SurfaceCode,
                code_distance: 7,
                physical_error_rate: 1e-3,
                error_correction_threshold: 0.01,
                logical_error_prefactor: 0.038,
            },
            timing: TimingConfig {
                cycle_time_us: 1.0,
                measurement_time_us: 0.5,
                classical_feedback_latency_us: 1.0,
            },
            factory: FactoryConfig::Distillation {
                count: 1,
                protocol: DistillationProtocol::FifteenToOne,
                cycles_per_round: 10,
                rounds: 3,
                abort_probability: 0.01,
            },
            injection: InjectionConfig {
                error_probability: 0.5,
                fixup_cost_cycles: 2,
            },
            routing: RoutingConfig::default(),
            buffer: BufferConfig {
                capacity: 4,
                preload: 0,
            },
        }
    }

    /// Build an n-op linear chain: op(0) → op(1) → … → op(n-1).
    pub fn chain_circuit(n: u32) -> ProfilerCircuit {
        let ops = (0..n)
            .map(|i| Operation {
                id: u64::from(i),
                kind: IrOpKind::Clifford,
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
            metadata: meta(),
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
    use pirx_ir::circuit::{
        CircuitMetadata, ConditionalActivation, Dependency, MeasurementHook, MeasurementOutcome,
        OpKind as IrOpKind, Operation, ProfilerCircuit,
    };
    use smallvec::smallvec;

    use super::{
        fixtures::{minimal_hw, validated},
        *,
    };

    // ── initial_ready_set ───────────────────────────────────────────────────

    #[test]
    fn initial_ready_set_roots_only() {
        let hw = minimal_hw();
        // A(0) and B(1) both → C(2). Only A and B are roots.
        let circuit = ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![0],
                    initially_active: true,
                },
                Operation {
                    id: 1,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![1],
                    initially_active: true,
                },
                Operation {
                    id: 2,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![0, 1],
                    initially_active: true,
                },
            ],
            deps: vec![Dependency { from: 0, to: 2 }, Dependency { from: 1, to: 2 }],
            qubit_count: 2,
            qubit_positions: None,
            hooks: vec![],
            metadata: CircuitMetadata {
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 0,
                rotation_count: 0,
                depth: 0,
            },
        };
        let dag = Dag::from_circuit(&validated(circuit), &hw)
            .map(|b| b.dag)
            .expect("valid");
        let roots = dag.initial_ready_set();
        assert_eq!(roots.len(), 2);
        for &k in &roots {
            assert_eq!(dag.adjacency.predecessor_count.get(k).copied(), Some(0));
        }
    }

    #[test]
    fn initial_ready_set_parallel() {
        let hw = minimal_hw();
        // Three independent ops — all are roots.
        let circuit = ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![0],
                    initially_active: true,
                },
                Operation {
                    id: 1,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![1],
                    initially_active: true,
                },
                Operation {
                    id: 2,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![2],
                    initially_active: true,
                },
            ],
            deps: vec![],
            qubit_count: 3,
            qubit_positions: None,
            hooks: vec![],
            metadata: CircuitMetadata {
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 0,
                rotation_count: 0,
                depth: 0,
            },
        };
        let dag = Dag::from_circuit(&validated(circuit), &hw)
            .map(|b| b.dag)
            .expect("valid");
        assert_eq!(dag.initial_ready_set().len(), 3);
    }

    #[test]
    fn inactive_op_excluded_from_initial_ready_set() {
        let hw = minimal_hw();
        let circuit = ProfilerCircuit {
            ops: vec![
                Operation {
                    id: 0,
                    kind: IrOpKind::Measurement { hook: Some(0) },
                    qubits: smallvec![0],
                    initially_active: true,
                },
                Operation {
                    id: 1,
                    kind: IrOpKind::Clifford,
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![],
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
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 0,
                rotation_count: 0,
                depth: 0,
            },
        };
        let dag = Dag::from_circuit(&validated(circuit), &hw)
            .map(|b| b.dag)
            .expect("valid");
        let ready = dag.initial_ready_set();
        assert_eq!(
            ready.len(),
            1,
            "only the active op should be in the ready set"
        );
    }
}
