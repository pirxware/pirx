//! DAG mutation primitives used during the simulation loop.

use smallvec::SmallVec;

use super::{
    Dag,
    kind::{OpData, OpKey, OpKind},
    ready_queue::FifoReadyQueue,
};

impl Dag {
    /// Decrement predecessor counts for all successors of `gate`.
    ///
    /// Any successor whose count reaches zero is pushed onto `queue` (it is
    /// now ready to execute).
    pub fn release_successors(&mut self, gate: OpKey, queue: &mut FifoReadyQueue) {
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
        queue: &mut FifoReadyQueue,
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
    pub fn inject_fixup(&mut self, gate: OpKey, queue: &mut FifoReadyQueue) -> OpKey {
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

    use super::super::fixtures::{chain_circuit, minimal_hw, validated};
    use crate::dag::{Dag, FifoReadyQueue, OpKind};

    // ── release_successors ──────────────────────────────────────────────────

    #[test]
    fn release_successors_decrements() {
        let hw = minimal_hw();
        // A → B → C
        let mut dag = Dag::from_circuit(&validated(chain_circuit(3)), &hw)
            .expect("valid")
            .dag;
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

    #[test]
    fn release_successors_skips_inactive_successor() {
        let hw = minimal_hw();
        // A(active) → B(inactive): release_successors(A) must NOT push B.
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
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 0,
                rotation_count: 0,
                depth: 0,
            },
        };
        let mut dag = Dag::from_circuit(&validated(circuit), &hw)
            .map(|b| b.dag)
            .expect("valid");
        let key_a = dag.initial_ready_set()[0];
        let mut queue = FifoReadyQueue::new();
        dag.release_successors(key_a, &mut queue);
        // B is inactive, so even though pred count hit 0, it must not be queued.
        assert!(queue.is_empty(), "inactive successor must not be pushed");
    }

    // ── inject_fixup ────────────────────────────────────────────────────────

    #[test]
    fn inject_fixup_rewires() {
        let hw = minimal_hw();
        // A(0) → B(1) → C(2)
        let mut dag = Dag::from_circuit(&validated(chain_circuit(3)), &hw)
            .expect("valid")
            .dag;

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
        let mut dag = Dag::from_circuit(&validated(chain_circuit(2)), &hw)
            .expect("valid")
            .dag;
        let key_a = dag.initial_ready_set()[0];
        let mut queue = FifoReadyQueue::new();
        let key_f = dag.inject_fixup(key_a, &mut queue);
        let fixup = dag.get(key_f).expect("fixup");
        assert!(fixup.active, "injected fixup must be active");
        // Predecessors map must contain an entry for the fixup.
        assert!(dag.adjacency.predecessors.get(key_f).is_some());
    }

    // ── activate_ops ────────────────────────────────────────────────────────

    #[test]
    fn activate_ops_with_completed_predecessors_pushes_to_queue() {
        let hw = minimal_hw();
        // A(active) → B(inactive)
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
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 0,
                rotation_count: 0,
                depth: 0,
            },
        };
        let mut dag = Dag::from_circuit(&validated(circuit), &hw)
            .map(|b| b.dag)
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
                name: "test".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 0,
                rotation_count: 0,
                depth: 0,
            },
        };
        let mut dag = Dag::from_circuit(&validated(circuit), &hw)
            .map(|b| b.dag)
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
