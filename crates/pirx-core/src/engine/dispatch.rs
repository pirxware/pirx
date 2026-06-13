//! Inner simulation loop — event processing, gate scheduling, factory restart.

use pirx_ir::circuit::{MeasurementHookId, MeasurementOutcome};
use rand::Rng as _;

use super::Engine;
use crate::{
    dag::{OpKey, OpKind, ReadyQueue},
    events::{EngineEvent, TimedEvent},
    factory::{FactoryModel, FactoryOutcome},
    routing::RoutingModel,
    trace::{SYNTHETIC_ID_FLAG, TraceEventKind},
};

/// Index into [`Engine::hook_table`] for a (hook_id, outcome) pair.
/// Layout: `hook_id * 2 + outcome_ordinal`. Zero and One map to 0 and 1.
pub(super) fn hook_table_index(hook_id: MeasurementHookId, outcome: MeasurementOutcome) -> usize {
    let ordinal = match outcome {
        MeasurementOutcome::Zero => 0,
        MeasurementOutcome::One => 1,
    };
    (hook_id as usize).saturating_mul(2).saturating_add(ordinal)
}

impl Engine {
    pub(super) fn process_event(&mut self, event: TimedEvent) {
        match event.event {
            EngineEvent::FactoryProduced { factory_id } => {
                self.trace
                    .record(event.cycle, TraceEventKind::FactoryProduced { factory_id });
                if self.buffer.try_enqueue() {
                    self.trace.record(
                        event.cycle,
                        TraceEventKind::BufferEnqueue {
                            occupancy: self.buffer.occupancy(),
                        },
                    );
                } else {
                    self.trace.record(event.cycle, TraceEventKind::BufferFull);
                }
                self.start_factory(factory_id, event.cycle);
            }
            EngineEvent::FactoryFailed { factory_id } => {
                self.trace
                    .record(event.cycle, TraceEventKind::FactoryFailed { factory_id });
                self.start_factory(factory_id, event.cycle);
            }
            EngineEvent::GateCompleted { gate } => {
                let gate_id = self.trace_id(gate);
                let kind = self.dag.get(gate).map(|op| op.kind);
                if kind == Some(OpKind::Fixup) {
                    self.trace.record(
                        event.cycle,
                        TraceEventKind::FixupCompleted { fixup: gate_id },
                    );
                } else {
                    self.trace
                        .record(event.cycle, TraceEventKind::GateCompleted { gate: gate_id });
                }
                self.completed_ops = self.completed_ops.saturating_add(1);
                if !self.hook_table.is_empty() {
                    self.completed_set.insert(gate, ());
                }
                self.complete_gate(gate, kind);
            }
            EngineEvent::HookActivation {
                gate,
                hook_id,
                outcome,
            } => {
                self.activate_hook(gate, hook_id, outcome);
            }
        }
    }

    /// Handle a gate that has finished executing.
    fn complete_gate(&mut self, gate: crate::dag::OpKey, kind: Option<OpKind>) {
        let Some(kind) = kind else {
            return;
        };

        let inject = match kind {
            OpKind::TGate | OpKind::Rotation { .. } => {
                self.rng.random::<f64>() < self.injection_error_probability
            }
            OpKind::Clifford | OpKind::Measurement { .. } | OpKind::Fixup => false,
        };

        if inject {
            let gate_id = self.trace_id(gate);
            self.trace.record(
                self.current_cycle,
                TraceEventKind::InjectionError { gate: gate_id },
            );
            let fixup_key = self.dag.inject_fixup(gate, &mut self.ready_set);
            let synthetic_id = SYNTHETIC_ID_FLAG | self.next_synthetic_id;
            self.next_synthetic_id = self.next_synthetic_id.saturating_add(1);
            self.key_to_op_id.insert(fixup_key, synthetic_id);
            self.trace.record(
                self.current_cycle,
                TraceEventKind::FixupInserted {
                    fixup: synthetic_id,
                    original: gate_id,
                },
            );
            // Fixup is now in ready_set. Count it so is_complete() stays correct.
            self.total_ops = self.total_ops.saturating_add(1);
        } else {
            if let OpKind::Measurement {
                hook: Some(hook_id),
            } = kind
            {
                self.dispatch_hook(gate, hook_id);
            }
            self.dag.release_successors(gate, &mut self.ready_set);
        }
    }

    /// Sample a measurement outcome and activate (or defer) the corresponding ops.
    ///
    /// `total_ops` is incremented immediately so the engine does not falsely
    /// terminate before a deferred `HookActivation` fires.
    fn dispatch_hook(&mut self, gate: OpKey, hook_id: MeasurementHookId) {
        let gate_id = self.trace_id(gate);

        let outcome = if self.rng.random::<bool>() {
            MeasurementOutcome::One
        } else {
            MeasurementOutcome::Zero
        };

        self.trace.record(
            self.current_cycle,
            TraceEventKind::MeasurementOutcome {
                gate: gate_id,
                outcome,
            },
        );

        // Count activated ops now so is_complete() waits for them.
        let idx = hook_table_index(hook_id, outcome);
        if let Some(to_activate) = self.hook_table.get(idx) {
            self.total_ops = self.total_ops.saturating_add(to_activate.len() as u64);
        }

        if self.feedback_delay == 0 {
            self.activate_hook(gate, hook_id, outcome);
        } else {
            self.event_queue.schedule(
                self.current_cycle.saturating_add(self.feedback_delay),
                EngineEvent::HookActivation {
                    gate,
                    hook_id,
                    outcome,
                },
            );
        }
    }

    /// Activate ops for a resolved measurement hook.
    fn activate_hook(
        &mut self,
        gate: OpKey,
        hook_id: MeasurementHookId,
        outcome: MeasurementOutcome,
    ) {
        let gate_id = self.trace_id(gate);

        let idx = hook_table_index(hook_id, outcome);
        let Some(to_activate) = self.hook_table.get(idx) else {
            return;
        };
        if to_activate.is_empty() {
            return;
        }

        let completed_set = &self.completed_set;
        self.dag.activate_ops(
            to_activate,
            &|k| completed_set.contains_key(k),
            &mut self.ready_set,
        );

        #[allow(clippy::cast_possible_truncation)]
        let activated_count = to_activate.len() as u32;

        self.trace.record(
            self.current_cycle,
            TraceEventKind::OpsActivated {
                gate: gate_id,
                activated_count,
            },
        );
    }

    /// Compute total gate cost including routing latency, emit routing trace
    /// events if the routing cost is non-zero.
    fn total_gate_cost(&mut self, gate: crate::dag::OpKey) -> u32 {
        let (base_cost, routing_cost) = if let Some(op) = self.dag.get(gate) {
            let rc = self
                .routing
                .latency(op.qubits.as_slice(), &self.position_index);
            (op.cycle_cost, rc)
        } else {
            (1, 0)
        };

        if routing_cost > 0 {
            let gate_id = self.trace_id(gate);
            self.trace.record(
                self.current_cycle,
                TraceEventKind::RoutingStarted { gate: gate_id },
            );
            self.trace.record(
                self.current_cycle,
                TraceEventKind::RoutingCompleted {
                    gate: gate_id,
                    latency: routing_cost,
                },
            );
        }

        base_cost.saturating_add(routing_cost)
    }

    /// Schedule all gates currently in the ready queue.
    pub(super) fn schedule_ready_gates(&mut self) {
        while let Some(gate) = self.ready_set.pop() {
            let gate_id = self.trace_id(gate);
            self.trace.record(
                self.current_cycle,
                TraceEventKind::GateReady { gate: gate_id },
            );

            let needs_magic_state = matches!(
                self.dag.get(gate).map(|op| op.kind),
                Some(OpKind::TGate) | Some(OpKind::Rotation { .. })
            );

            if needs_magic_state {
                if self.buffer.try_dequeue() {
                    self.trace.record(
                        self.current_cycle,
                        TraceEventKind::BufferDequeue {
                            occupancy: self.buffer.occupancy(),
                        },
                    );
                    self.trace.record(
                        self.current_cycle,
                        TraceEventKind::GateServed {
                            gate: gate_id,
                            wait: 0,
                        },
                    );
                    self.schedule_gate_completion(gate);
                } else {
                    self.trace.record(
                        self.current_cycle,
                        TraceEventKind::GateStalled { gate: gate_id },
                    );
                    self.stalled_gates.push_back((gate, self.current_cycle));
                }
            } else {
                self.trace.record(
                    self.current_cycle,
                    TraceEventKind::GateScheduled { gate: gate_id },
                );
                self.schedule_gate_completion(gate);
            }
        }
    }

    /// Schedule the `GateCompleted` event for `gate`, accounting for its base
    /// cost and any routing latency.
    fn schedule_gate_completion(&mut self, gate: crate::dag::OpKey) {
        let cost = self.total_gate_cost(gate);
        self.event_queue.schedule(
            self.current_cycle.saturating_add(u64::from(cost)),
            EngineEvent::GateCompleted { gate },
        );
    }

    /// Serve as many stalled gates as the buffer allows (FIFO order).
    pub(super) fn try_serve_stalled_gates(&mut self) {
        while let Some(&(gate, stall_cycle)) = self.stalled_gates.front() {
            if !self.buffer.try_dequeue() {
                break;
            }
            self.stalled_gates.pop_front();

            let wait =
                u32::try_from(self.current_cycle.saturating_sub(stall_cycle)).unwrap_or(u32::MAX);

            self.trace.record(
                self.current_cycle,
                TraceEventKind::BufferDequeue {
                    occupancy: self.buffer.occupancy(),
                },
            );
            self.trace.record(
                self.current_cycle,
                TraceEventKind::GateServed {
                    gate: self.trace_id(gate),
                    wait,
                },
            );
            self.schedule_gate_completion(gate);
        }
    }

    /// Schedule the next production event for factory `factory_id`.
    pub(super) fn start_factory(&mut self, factory_id: u16, current_cycle: u64) {
        let idx = usize::from(factory_id);
        if idx >= self.factories.len() {
            return;
        }
        // Split borrow: factories[idx] and factory_rngs[idx] are different fields.
        // idx is bounds-checked above, so indexing cannot panic.
        #[allow(clippy::indexing_slicing)]
        let outcome =
            self.factories[idx].schedule_production(current_cycle, &mut self.factory_rngs[idx]);
        let (event_cycle, event) = match outcome {
            FactoryOutcome::Produced { completion_cycle } => (
                completion_cycle,
                EngineEvent::FactoryProduced { factory_id },
            ),
            FactoryOutcome::Failed { failure_cycle } => {
                (failure_cycle, EngineEvent::FactoryFailed { factory_id })
            }
        };
        self.trace
            .record(current_cycle, TraceEventKind::FactoryStarted { factory_id });
        self.event_queue.schedule(event_cycle, event);
    }
}
