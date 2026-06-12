//! Circuit dependency graph for the DES engine.
//!
//! `Dag` holds the operation arena, adjacency maps, and rotation-angle dedup
//! table. Simulation state (current cycle, event queue, stalled gates) lives
//! in the Engine, not here. Mutation is exposed only via
//! [`Dag::release_successors`] and [`Dag::inject_fixup`].

use std::collections::VecDeque;

use pirx_hw::model::HardwareModel;
use pirx_ir::circuit::{MeasurementHookId, OpId, OpKind as IrOpKind, ProfilerCircuit, QubitId};
use serde::{Deserialize, Serialize};
use slotmap::{SecondaryMap, SlotMap, new_key_type};
use smallvec::SmallVec;
use thiserror::Error;

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

/// DAG adjacency — held separately from node data for cache locality.
///
/// Node data is hot during scheduling; adjacency is hot during ready-set
/// computation. Separate `SecondaryMap`s let them occupy different cache lines.
#[derive(Debug, Serialize, Deserialize)]
pub struct DagAdjacency {
    /// Successor keys for each node.
    ///
    /// `SmallVec<[OpKey; 4]>` — 32 bytes inline, no heap allocation for the
    /// common 1-4 successors case.
    pub successors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>>,
    /// Predecessor keys for each node. Used by [`Dag::activate_ops`] to
    /// recompute effective predecessor counts after activation.
    pub predecessors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>>,
    /// Number of predecessors not yet completed. Decremented by
    /// [`Dag::release_successors`]. Reaches zero when the node is ready.
    pub predecessor_count: SecondaryMap<OpKey, u32>,
}

/// Errors that can occur during DAG construction.
#[derive(Debug, Error)]
pub enum DagError {
    #[error("circuit has no operations")]
    EmptyCircuit,

    #[error("dependency references a non-existent operation ID")]
    DanglingDependency,

    #[error("dependency graph contains a cycle")]
    CyclicDag,

    #[error("too many distinct rotation angles: {0} (maximum 65535)")]
    TooManyDistinctAngles(usize),
}

// ── Ready queue ──────────────────────────────────────────────────────────────

/// Interface for the ready-gate queue.
///
/// The default implementation is [`FifoReadyQueue`]. The trait exists so that
/// future priority-scheduling policies (critical-path-first, T-gate-first) can
/// be swapped in without changing the engine loop.
pub trait ReadyQueue {
    fn push(&mut self, key: OpKey);
    fn pop(&mut self) -> Option<OpKey>;
    fn is_empty(&self) -> bool;
    fn len(&self) -> usize;
}

/// FIFO ready queue — the default scheduling policy.
///
/// Gates that become ready in the same cycle are served in insertion order,
/// matching the priority-list scheduling model and ensuring determinism under
/// a fixed seed.
#[derive(Debug, Serialize, Deserialize)]
pub struct FifoReadyQueue {
    inner: VecDeque<OpKey>,
}

impl FifoReadyQueue {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(capacity),
        }
    }
}

impl Default for FifoReadyQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadyQueue for FifoReadyQueue {
    fn push(&mut self, key: OpKey) {
        self.inner.push_back(key);
    }

    fn pop(&mut self) -> Option<OpKey> {
        self.inner.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

// ── DAG ──────────────────────────────────────────────────────────────────────

/// Circuit dependency graph — the engine's structural view of the circuit.
///
/// Owns the operation arena, adjacency maps, and the rotation-angle dedup
/// table. The Engine drives simulation; the DAG provides operations, edges,
/// and the two mutation primitives used during the simulation loop.
///
/// Callers should prefer [`Dag::release_successors`] and [`Dag::inject_fixup`]
/// over direct mutation of `adjacency`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Dag {
    ops: SlotMap<OpKey, OpData>,
    /// Adjacency maps — `pub` so the Engine can read them directly for
    /// trace-event generation without a method-call overhead.
    pub adjacency: DagAdjacency,
    /// Deduped rotation angles. `OpKind::Rotation { angle_index }` indexes here.
    angle_table: Vec<f64>,
    /// QEC cycles per injected fixup node (from `HardwareModel::injection`).
    fixup_cost_cycles: u32,
}

impl Dag {
    /// Build a DAG from a validated IR circuit.
    ///
    /// Returns the DAG and a map from IR `OpId` to arena `OpKey`, needed by
    /// the engine to resolve measurement hook targets.
    ///
    /// `circuit` must be non-empty and acyclic. [`pirx_ir::validate`] checks
    /// both before circuits cross the crate boundary; `from_circuit` repeats
    /// the acyclicity check as defence-in-depth for circuits constructed
    /// without going through the validator.
    ///
    /// `hw` is used only for `injection.fixup_cost_cycles`.
    pub fn from_circuit(
        circuit: &ProfilerCircuit,
        hw: &HardwareModel,
    ) -> Result<(Self, std::collections::HashMap<OpId, OpKey>), DagError> {
        if circuit.ops.is_empty() {
            return Err(DagError::EmptyCircuit);
        }

        let n = circuit.ops.len();
        let mut ops: SlotMap<OpKey, OpData> = SlotMap::with_capacity_and_key(n);
        let mut successors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>> =
            SecondaryMap::with_capacity(n);
        let mut predecessors: SecondaryMap<OpKey, SmallVec<[OpKey; 4]>> =
            SecondaryMap::with_capacity(n);
        let mut predecessor_count: SecondaryMap<OpKey, u32> = SecondaryMap::with_capacity(n);
        let mut angle_table: Vec<f64> = Vec::new();

        // Map IR OpId → arena OpKey for building adjacency.
        let mut id_to_key: std::collections::HashMap<OpId, OpKey> =
            std::collections::HashMap::with_capacity(n);

        for op in &circuit.ops {
            let kind = ir_kind_to_engine(&op.kind, &mut angle_table)?;
            let key = ops.insert(OpData {
                kind,
                qubits: op.qubits.clone(),
                // Default: 1 QEC round per gate. The Engine applies timing
                // refinements (measurement latency, routing overhead) at
                // scheduling time.
                cycle_cost: 1,
                active: op.initially_active,
            });
            successors.insert(key, SmallVec::new());
            predecessors.insert(key, SmallVec::new());
            predecessor_count.insert(key, 0);
            id_to_key.insert(op.id, key);
        }

        // Build adjacency from the dependency list.
        for dep in &circuit.deps {
            let &from_key = id_to_key
                .get(&dep.from)
                .ok_or(DagError::DanglingDependency)?;
            let &to_key = id_to_key.get(&dep.to).ok_or(DagError::DanglingDependency)?;
            if let Some(succs) = successors.get_mut(from_key) {
                succs.push(to_key);
            }
            if let Some(preds) = predecessors.get_mut(to_key) {
                preds.push(from_key);
            }
            if let Some(count) = predecessor_count.get_mut(to_key) {
                *count += 1;
            }
        }

        // Defence-in-depth: detect cycles via Kahn's topological sort.
        // pirx-ir runs this before circuits leave that crate; this fires only
        // for circuits constructed without going through validate().
        detect_cycle(&ops, &successors, &predecessor_count, n)?;

        Ok((
            Self {
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
        ))
    }

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

    /// Decrement predecessor counts for all successors of `gate`.
    ///
    /// Any successor whose count reaches zero is pushed onto `queue` (it is
    /// now ready to execute).
    pub fn release_successors(&mut self, gate: OpKey, queue: &mut dyn ReadyQueue) {
        // Split-borrow: ops, successors, and predecessor_count are disjoint
        // fields; Rust NLL tracks them independently.
        let ops = &self.ops;
        let succs = &self.adjacency.successors;
        let pred = &mut self.adjacency.predecessor_count;

        if let Some(succs) = succs.get(gate) {
            for &succ in succs.iter() {
                if let Some(count) = pred.get_mut(succ) {
                    let was_positive = *count > 0;
                    *count = count.saturating_sub(1);
                    if was_positive && *count == 0 && ops.get(succ).is_some_and(|op| op.active) {
                        queue.push(succ);
                    }
                }
            }
        }
    }

    /// Activate pre-allocated ops after a measurement outcome.
    ///
    /// Recomputes effective predecessor_count by checking how many
    /// predecessors are not yet completed. Pushes newly-ready ops to `queue`.
    ///
    /// `completed` is a callback that returns `true` if the given op has
    /// already finished executing. The DAG does not track completion state —
    /// the engine does.
    pub fn activate_ops(
        &mut self,
        op_keys: &[OpKey],
        completed: &impl Fn(OpKey) -> bool,
        queue: &mut dyn ReadyQueue,
    ) {
        for &key in op_keys {
            if let Some(op) = self.ops.get_mut(key) {
                op.active = true;
            }
            // Recompute live predecessor count from the actual predecessors list.
            #[allow(clippy::cast_possible_truncation)]
            let live_pending = self.adjacency.predecessors.get(key).map_or(0u32, |preds| {
                preds.iter().filter(|&&p| !completed(p)).count() as u32
            });
            self.adjacency.predecessor_count.insert(key, live_pending);
            if live_pending == 0 {
                queue.push(key);
            }
        }
    }

    /// Insert a fixup node immediately after `gate` and push it onto `queue`.
    ///
    /// ```text
    /// Before: gate → [S1, S2, …]
    /// After:  gate → fixup → [S1, S2, …]
    /// ```
    ///
    /// Predecessor counts for S1, S2, … are unchanged — only the identity of
    /// their predecessor changes, not the count. Fixup's `predecessor_count`
    /// is set to 0 because `gate` has already completed, making fixup
    /// immediately ready.
    pub fn inject_fixup(&mut self, gate: OpKey, queue: &mut dyn ReadyQueue) -> OpKey {
        debug_assert!(
            self.ops.contains_key(gate),
            "inject_fixup: invalid gate key"
        );

        let qubits = self
            .ops
            .get(gate)
            .map_or_else(SmallVec::new, |op| op.qubits.clone());

        let fixup = self.ops.insert(OpData {
            kind: OpKind::Fixup,
            qubits,
            cycle_cost: self.fixup_cost_cycles,
            active: true, // fixups are always immediately active
        });

        // Move gate's successors to fixup — take, don't clone (hot path).
        let gate_succs: SmallVec<[OpKey; 4]> = self
            .adjacency
            .successors
            .get_mut(gate)
            .map(std::mem::take)
            .unwrap_or_default();
        self.adjacency.successors.insert(fixup, gate_succs);

        // Gate's successor list is now empty (taken above) — point it at fixup.
        if let Some(s) = self.adjacency.successors.get_mut(gate) {
            s.push(fixup);
        }

        // Fixup is immediately ready (gate already completed when this is called).
        self.adjacency.predecessor_count.insert(fixup, 0);
        self.adjacency.predecessors.insert(fixup, SmallVec::new());
        queue.push(fixup);

        fixup
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

// ── Helpers ──────────────────────────────────────────────────────────────────

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

/// Kahn's topological sort — detects cycles without consuming the adjacency data.
///
/// Operates on a copy of `predecessor_count`; the original is untouched.
fn detect_cycle(
    ops: &SlotMap<OpKey, OpData>,
    successors: &SecondaryMap<OpKey, SmallVec<[OpKey; 4]>>,
    predecessor_count: &SecondaryMap<OpKey, u32>,
    expected: usize,
) -> Result<(), DagError> {
    let mut in_deg: SecondaryMap<OpKey, u32> = SecondaryMap::new();
    for (k, &c) in predecessor_count.iter() {
        in_deg.insert(k, c);
    }

    let mut queue: VecDeque<OpKey> = ops
        .keys()
        .filter(|&k| in_deg.get(k).copied().unwrap_or(0) == 0)
        .collect();

    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(succs) = successors.get(node) {
            for &succ in succs.iter() {
                if let Some(count) = in_deg.get_mut(succ) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        queue.push_back(succ);
                    }
                }
            }
        }
    }

    if visited == expected {
        Ok(())
    } else {
        Err(DagError::CyclicDag)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use pirx_hw::model::{
        BufferConfig, DistillationProtocol, FactoryConfig, HardwareModel, InjectionConfig,
        MetaConfig, QecConfig, TimingConfig,
    };
    use pirx_hw::{CodeType, RoutingConfig};
    use pirx_ir::circuit::{
        CircuitMetadata, Dependency, OpKind as IrOpKind, Operation, ProfilerCircuit,
    };
    use smallvec::smallvec;

    use super::*;

    // ── Fixtures ────────────────────────────────────────────────────────────

    fn meta() -> CircuitMetadata {
        CircuitMetadata {
            name: "test".into(),
            source_framework: "test".into(),
            t_count: 0,
            clifford_count: 0,
            rotation_count: 0,
            depth: 0,
        }
    }

    fn minimal_hw() -> HardwareModel {
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
    fn chain_circuit(n: u32) -> ProfilerCircuit {
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

    // ── Construction ────────────────────────────────────────────────────────

    #[test]
    fn from_circuit_empty_rejected() {
        let hw = minimal_hw();
        let circuit = ProfilerCircuit {
            ops: vec![],
            deps: vec![],
            qubit_count: 0,
            qubit_positions: None,
            hooks: vec![],
            metadata: meta(),
        };
        assert!(matches!(
            Dag::from_circuit(&circuit, &hw).map(|(d, _)| d),
            Err(DagError::EmptyCircuit)
        ));
    }

    #[test]
    fn from_circuit_cycle_rejected() {
        let hw = minimal_hw();
        // A(0) → B(1) → A(0) — cycle bypasses pirx-ir validation intentionally.
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
                    qubits: smallvec![0],
                    initially_active: true,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }, Dependency { from: 1, to: 0 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: meta(),
        };
        assert!(matches!(
            Dag::from_circuit(&circuit, &hw).map(|(d, _)| d),
            Err(DagError::CyclicDag)
        ));
    }

    #[test]
    fn from_circuit_simple_chain() {
        let hw = minimal_hw();
        let (dag, _) = Dag::from_circuit(&chain_circuit(2), &hw).expect("valid chain");
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
            metadata: meta(),
        };
        let dag = Dag::from_circuit(&circuit, &hw)
            .map(|(d, _)| d)
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
            metadata: meta(),
        };
        let dag = Dag::from_circuit(&circuit, &hw)
            .map(|(d, _)| d)
            .expect("valid");
        assert_eq!(dag.initial_ready_set().len(), 3);
    }

    // ── release_successors ──────────────────────────────────────────────────

    #[test]
    fn release_successors_decrements() {
        let hw = minimal_hw();
        // A → B → C
        let (mut dag, _) = Dag::from_circuit(&chain_circuit(3), &hw).expect("valid");
        let roots = dag.initial_ready_set();
        assert_eq!(roots.len(), 1);
        let key_a = roots[0];

        let key_b = dag
            .adjacency
            .successors
            .get(key_a)
            .and_then(|s| s.first())
            .copied()
            .expect("B exists");

        assert_eq!(dag.adjacency.predecessor_count.get(key_b).copied(), Some(1));

        let mut queue = FifoReadyQueue::new();
        dag.release_successors(key_a, &mut queue);

        assert_eq!(dag.adjacency.predecessor_count.get(key_b).copied(), Some(0));
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop(), Some(key_b));
    }

    // ── inject_fixup ────────────────────────────────────────────────────────

    #[test]
    fn inject_fixup_rewires() {
        let hw = minimal_hw();
        // A(0) → B(1) → C(2)
        let (mut dag, _) = Dag::from_circuit(&chain_circuit(3), &hw).expect("valid");

        let key_a = dag.initial_ready_set()[0];
        let key_b = dag
            .adjacency
            .successors
            .get(key_a)
            .and_then(|s| s.first())
            .copied()
            .expect("B exists");
        let key_c = dag
            .adjacency
            .successors
            .get(key_b)
            .and_then(|s| s.first())
            .copied()
            .expect("C exists");

        // C starts with predecessor_count == 1 (only B).
        let c_pred_before = dag
            .adjacency
            .predecessor_count
            .get(key_c)
            .copied()
            .expect("C pred count");
        assert_eq!(c_pred_before, 1);

        let mut queue = FifoReadyQueue::new();
        let key_f = dag.inject_fixup(key_b, &mut queue);

        // B → [F]
        let b_succs = dag.adjacency.successors.get(key_b).expect("B successors");
        assert_eq!(b_succs.as_slice(), &[key_f]);

        // F → [C]
        let f_succs = dag.adjacency.successors.get(key_f).expect("F successors");
        assert_eq!(f_succs.as_slice(), &[key_c]);

        // C's predecessor_count unchanged — F replaced B as its predecessor.
        assert_eq!(dag.adjacency.predecessor_count.get(key_c).copied(), Some(1));

        // Fixup predecessor_count == 0 (immediately ready).
        assert_eq!(dag.adjacency.predecessor_count.get(key_f).copied(), Some(0));

        // Fixup is in the ready queue.
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.pop(), Some(key_f));

        // Fixup node has the correct cycle_cost from hw.injection.
        let fixup_data = dag.get(key_f).expect("fixup data");
        assert_eq!(fixup_data.cycle_cost, 2); // minimal_hw fixup_cost_cycles = 2
        assert!(matches!(fixup_data.kind, OpKind::Fixup));
    }

    #[test]
    fn inject_fixup_sets_active_true() {
        let hw = minimal_hw();
        let (mut dag, _) = Dag::from_circuit(&chain_circuit(2), &hw).expect("valid");
        let key_a = dag.initial_ready_set()[0];
        let mut queue = FifoReadyQueue::new();
        let key_f = dag.inject_fixup(key_a, &mut queue);
        let fixup = dag.get(key_f).expect("fixup");
        assert!(fixup.active, "injected fixup must be active");
        // Predecessors map must contain an entry for the fixup.
        assert!(dag.adjacency.predecessors.get(key_f).is_some());
    }

    // ── active flag / activate_ops ──────────────────────────────────────────

    #[test]
    fn inactive_op_excluded_from_initial_ready_set() {
        let hw = minimal_hw();
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
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: meta(),
        };
        let dag = Dag::from_circuit(&circuit, &hw)
            .map(|(d, _)| d)
            .expect("valid");
        let ready = dag.initial_ready_set();
        assert_eq!(
            ready.len(),
            1,
            "only the active op should be in the ready set"
        );
    }

    #[test]
    fn release_successors_skips_inactive_successor() {
        let hw = minimal_hw();
        // A(active) → B(inactive): release_successors(A) must NOT push B.
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
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: meta(),
        };
        let mut dag = Dag::from_circuit(&circuit, &hw)
            .map(|(d, _)| d)
            .expect("valid");
        let key_a = dag.initial_ready_set()[0];
        let mut queue = FifoReadyQueue::new();
        dag.release_successors(key_a, &mut queue);
        // B is inactive, so even though pred count hit 0, it must not be queued.
        assert!(queue.is_empty(), "inactive successor must not be pushed");
    }

    #[test]
    fn activate_ops_with_completed_predecessors_pushes_to_queue() {
        let hw = minimal_hw();
        // A(active) → B(inactive)
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
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: meta(),
        };
        let mut dag = Dag::from_circuit(&circuit, &hw)
            .map(|(d, _)| d)
            .expect("valid");

        // Find key_b (the inactive op).
        let key_a = dag.initial_ready_set()[0];
        let key_b = dag
            .adjacency
            .successors
            .get(key_a)
            .and_then(|s| s.first())
            .copied()
            .expect("B exists");

        // A is already completed.
        let mut queue = FifoReadyQueue::new();
        dag.activate_ops(&[key_b], &|k| k == key_a, &mut queue);

        assert!(dag.get(key_b).unwrap().active, "B must now be active");
        assert_eq!(
            dag.adjacency.predecessor_count.get(key_b).copied(),
            Some(0),
            "all predecessors completed → count must be 0"
        );
        assert_eq!(queue.len(), 1, "B must be pushed to queue");
        assert_eq!(queue.pop(), Some(key_b));
    }

    #[test]
    fn activate_ops_with_live_predecessors_not_pushed() {
        let hw = minimal_hw();
        // A(active) → B(inactive), A is NOT completed.
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
                    qubits: smallvec![0],
                    initially_active: false,
                },
            ],
            deps: vec![Dependency { from: 0, to: 1 }],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: meta(),
        };
        let mut dag = Dag::from_circuit(&circuit, &hw)
            .map(|(d, _)| d)
            .expect("valid");
        let key_a = dag.initial_ready_set()[0];
        let key_b = dag
            .adjacency
            .successors
            .get(key_a)
            .and_then(|s| s.first())
            .copied()
            .expect("B exists");

        // A is NOT completed — nothing_completed returns false for all.
        let mut queue = FifoReadyQueue::new();
        dag.activate_ops(&[key_b], &|_| false, &mut queue);

        assert!(dag.get(key_b).unwrap().active, "B must now be active");
        assert_eq!(
            dag.adjacency.predecessor_count.get(key_b).copied(),
            Some(1),
            "A is still live → count must be 1"
        );
        assert!(
            queue.is_empty(),
            "B must NOT be pushed (has live predecessor)"
        );
    }
}
