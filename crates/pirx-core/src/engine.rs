//! Discrete-event simulation engine.
//!
//! `Engine` drives the circuit DAG through factory events, gate scheduling,
//! magic state consumption, stall tracking, and injection error recovery.
//! All stochastic decisions flow through an explicit `ChaCha12Rng` seeded from
//! [`EngineConfig::seed`], ensuring full reproducibility (same seed → same trace).

use std::collections::VecDeque;

use pirx_hw::{RoutingConfig, model::HardwareModel};
use pirx_ir::{
    ValidatedCircuit,
    circuit::{MeasurementHookId, MeasurementOutcome, OpId},
};
use rand::{Rng as _, SeedableRng};
use rand_chacha::ChaCha12Rng;
use slotmap::SecondaryMap;
use smallvec::SmallVec;
use thiserror::Error;

use crate::{
    buffer::MagicStateBuffer,
    dag::{Dag, DagBuild, DagError, FifoReadyQueue, OpKey, OpKind, ReadyQueue},
    events::{EngineEvent, EventQueue, TimedEvent},
    factory::{FactoryError, FactoryModel, FactoryOutcome, create_factories},
    routing,
    trace::{SYNTHETIC_ID_FLAG, Trace, TraceCollector, TraceEventKind},
};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for [`Engine`] construction.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    /// RNG seed. Same seed + same inputs = identical trace, always.
    pub seed: u64,
    /// Maximum simulation cycles. `None` = run to completion.
    /// When hit, the engine stops and the trace records `truncated: true`.
    pub max_cycles: Option<u64>,
}

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors that can occur during engine construction.
///
/// All validation happens inside [`Engine::new`]. If `new` returns `Ok`,
/// the simulation is guaranteed to run to completion without error.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("too many distinct rotation angles: {0} (maximum 65535)")]
    TooManyRotationAngles(usize),

    #[error("hardware model has zero factories")]
    NoFactories,

    #[error("buffer capacity is zero")]
    ZeroBuffer,

    #[error("factory creation failed: {0}")]
    FactoryCreation(#[from] FactoryError),

    #[error("manhattan routing requires qubit_positions in circuit")]
    MissingQubitPositions,

    #[error("measurement hook {hook_id} references non-existent op {op_id}")]
    DanglingHookTarget {
        hook_id: MeasurementHookId,
        op_id: OpId,
    },

    #[error("internal DAG error: {0}")]
    Internal(String),
}

impl From<DagError> for EngineError {
    fn from(err: DagError) -> Self {
        match err {
            DagError::TooManyDistinctAngles(n) => Self::TooManyRotationAngles(n),
            DagError::Internal(msg) => Self::Internal(msg),
        }
    }
}

// ── Hook dispatch ────────────────────────────────────────────────────────────

/// Index into [`Engine::hook_table`] for a (hook_id, outcome) pair.
/// Layout: `hook_id * 2 + outcome_ordinal`. Zero and One map to 0 and 1.
fn hook_table_index(hook_id: MeasurementHookId, outcome: MeasurementOutcome) -> usize {
    let ordinal = match outcome {
        MeasurementOutcome::Zero => 0,
        MeasurementOutcome::One => 1,
    };
    (hook_id as usize).saturating_mul(2).saturating_add(ordinal)
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Discrete-event simulation engine.
///
/// Drives the circuit DAG through factory events, gate scheduling, magic state
/// consumption, stall tracking, and injection error recovery. Produces a
/// [`Trace`] capturing every state transition for post-hoc analysis.
///
/// Construction validates all inputs. If [`Engine::new`] returns `Ok`,
/// [`Engine::run`] is guaranteed to terminate.
pub struct Engine {
    // Circuit state
    dag: Dag,
    ready_set: Box<dyn ReadyQueue>,

    // Hardware state
    injection_error_probability: f64,
    factories: Vec<Box<dyn FactoryModel>>,
    buffer: MagicStateBuffer,
    routing: Box<dyn routing::RoutingModel>,
    position_index: Vec<(u32, u32)>,

    // Simulation state
    event_queue: EventQueue,
    current_cycle: u64,
    rng: ChaCha12Rng,
    factory_rngs: Vec<ChaCha12Rng>,
    seed: u64,
    max_cycles: Option<u64>,

    // Stalled T-gates: ready but waiting for a magic state.
    // Each entry pairs the gate key with the cycle it stalled, replacing the
    // former HashMap<OpKey, u64> side-table with zero per-stall hashing.
    stalled_gates: VecDeque<(OpKey, u64)>,

    // Measurement hook dispatch — flat table indexed by hook_id * 2 + outcome.
    // Pre-resolved at construction so the hot loop does a single Vec::get.
    hook_table: Vec<SmallVec<[OpKey; 2]>>,
    /// Tracks completed ops for dag.activate_ops() predecessor recomputation.
    completed_set: SecondaryMap<OpKey, ()>,

    // Termination tracking (total_ops grows when fixups are injected or hooks activate)
    total_ops: u64,
    completed_ops: u64,

    // Stable trace IDs: maps arena keys → IR OpId (or synthetic fixup ID).
    key_to_op_id: SecondaryMap<OpKey, u64>,
    next_synthetic_id: u64,

    // Trace collection (append-only, pre-allocated)
    trace: TraceCollector,
}

impl Engine {
    /// Build and validate the engine from a circuit, hardware model, and config.
    ///
    /// Constructs the DAG, creates factory instances, initializes the buffer
    /// and RNG, seeds the initial ready queue, schedules initial factory events,
    /// and pre-allocates the trace collector.
    pub fn new(
        circuit: &ValidatedCircuit,
        hw: &HardwareModel,
        config: EngineConfig,
    ) -> Result<Self, EngineError> {
        // Hardware validation before the (more expensive) DAG build.
        if hw.factory.count() == 0 {
            return Err(EngineError::NoFactories);
        }
        if hw.buffer.capacity == 0 {
            return Err(EngineError::ZeroBuffer);
        }

        let n_ops = circuit.ops.len();

        // Build DAG from the validated circuit.
        // id_to_key maps IR OpId → arena OpKey for hook target resolution.
        let DagBuild { dag, id_to_key } = Dag::from_circuit(circuit, hw)?;
        // Only count initially active ops; inactive ops enter via hook activation.
        let total_ops = dag.active_op_count() as u64;

        // Reverse map: arena key → IR OpId for stable trace event IDs.
        let mut key_to_op_id = SecondaryMap::with_capacity(n_ops.saturating_add(n_ops / 2));
        for (&op_id, &key) in &id_to_key {
            key_to_op_id.insert(key, op_id);
        }

        // Validate routing compatibility.
        if matches!(hw.routing, RoutingConfig::Manhattan { .. })
            && circuit.qubit_positions.is_none()
        {
            return Err(EngineError::MissingQubitPositions);
        }
        let routing_model = routing::from_config(&hw.routing);
        let position_index = routing::build_position_index(
            circuit.qubit_positions.as_deref().unwrap_or(&[]),
            circuit.qubit_count,
        );

        // Create factory instances from hardware config.
        let factories = create_factories(&hw.factory, &hw.qec)?;
        let factory_count = factories.len();

        // Initialize buffer.
        let buffer = MagicStateBuffer::new(hw.buffer.capacity, hw.buffer.preload);

        // Master RNG for injection errors and measurement hooks.
        let rng = ChaCha12Rng::seed_from_u64(config.seed);

        // Per-factory child RNGs: deterministic derivation from master seed
        // so factory N's sequence is stable regardless of total factory count.
        let mut factory_rngs: Vec<ChaCha12Rng> = (0..factory_count)
            .map(|i| {
                let factory_seed = config
                    .seed
                    .wrapping_add(i as u64)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15);
                ChaCha12Rng::seed_from_u64(factory_seed)
            })
            .collect();

        // Build initial ready set: all ops with predecessor_count == 0.
        let mut ready_set = Box::new(FifoReadyQueue::with_capacity(n_ops)) as Box<dyn ReadyQueue>;
        for &key in &dag.initial_ready_set() {
            ready_set.push(key);
        }

        // Capacity hint: ~5 events per gate + 3 per factory production round.
        let capacity_hint = n_ops
            .saturating_mul(5)
            .saturating_add(factory_count.saturating_mul(3));
        let mut trace = TraceCollector::new(capacity_hint);

        // Build hook dispatch table: flat Vec indexed by hook_id * 2 + outcome.
        // Pre-resolved so the hot loop does a single Vec::get, zero allocation.
        let table_len = circuit
            .hooks
            .iter()
            .map(|h| (h.id as usize).saturating_add(1))
            .max()
            .unwrap_or(0)
            .saturating_mul(2);
        let mut hook_table: Vec<SmallVec<[OpKey; 2]>> = vec![SmallVec::new(); table_len];
        for hook in &circuit.hooks {
            for activation in &hook.activations {
                let mut op_keys = SmallVec::<[OpKey; 2]>::new();
                for &op_id in &activation.ops_to_activate {
                    let &key = id_to_key
                        .get(&op_id)
                        .ok_or(EngineError::DanglingHookTarget {
                            hook_id: hook.id,
                            op_id,
                        })?;
                    op_keys.push(key);
                }
                let idx = hook_table_index(hook.id, activation.outcome);
                if let Some(slot) = hook_table.get_mut(idx) {
                    *slot = op_keys;
                }
            }
        }

        // Schedule initial factory events and record FactoryStarted at cycle 0.
        let mut event_queue = EventQueue::new();
        for (id, factory) in factories.iter().enumerate() {
            // factory_id: u16 per design — 65 535 max, more than any real architecture.
            #[allow(clippy::cast_possible_truncation)]
            let factory_id = id as u16;
            trace.record(0, TraceEventKind::FactoryStarted { factory_id });
            #[allow(clippy::indexing_slicing)]
            let outcome = factory.schedule_production(0, &mut factory_rngs[id]);
            let (event_cycle, event) = match outcome {
                FactoryOutcome::Produced { completion_cycle } => (
                    completion_cycle,
                    EngineEvent::FactoryProduced { factory_id },
                ),
                FactoryOutcome::Failed { failure_cycle } => {
                    (failure_cycle, EngineEvent::FactoryFailed { factory_id })
                }
            };
            event_queue.schedule(event_cycle, event);
        }

        Ok(Self {
            dag,
            ready_set,
            injection_error_probability: hw.injection.error_probability,
            factories,
            buffer,
            routing: routing_model,
            position_index,
            event_queue,
            current_cycle: 0,
            rng,
            factory_rngs,
            seed: config.seed,
            max_cycles: config.max_cycles,
            #[allow(clippy::cast_possible_truncation)]
            stalled_gates: VecDeque::with_capacity(
                circuit.metadata.t_count.min(n_ops as u64) as usize
            ),
            hook_table,
            completed_set: SecondaryMap::with_capacity(
                n_ops.saturating_add(n_ops / 2).saturating_add(16),
            ),
            key_to_op_id,
            next_synthetic_id: 0,
            total_ops,
            completed_ops: 0,
            trace,
        })
    }

    /// Run to completion or until `max_cycles` is reached, returning the sealed [`Trace`].
    ///
    /// Consumes the engine — an engine cannot be run twice.
    /// If `max_cycles` is set and the limit is reached before all ops complete,
    /// the returned trace has `truncated: true`.
    pub fn run(mut self) -> Trace {
        while !self.is_complete() {
            if let Some(max) = self.max_cycles {
                // DES jumps between event cycles — check the *next* event's cycle
                // before processing it, not current_cycle (which lags by one step).
                if self.event_queue.peek_cycle().is_some_and(|c| c >= max) {
                    return self.trace.finish_truncated(self.seed, self.current_cycle);
                }
            }
            self.step();
        }
        self.trace.finish(self.seed, self.current_cycle)
    }

    /// Advance the simulation by one step (one event-cycle).
    ///
    /// 1. Advance `current_cycle` to the next queued event cycle.
    /// 2. Process every event at that cycle.
    /// 3. Serve stalled T-gates from the buffer (FIFO).
    /// 4. Schedule all newly ready gates.
    pub fn step(&mut self) {
        let Some(cycle) = self.event_queue.peek_cycle() else {
            // No pending events — factories are always running, so this only
            // occurs after all ops complete. The run() loop guards on is_complete().
            return;
        };
        self.current_cycle = cycle;

        // Phase 1: process all events at this cycle.
        while self.event_queue.peek_cycle() == Some(cycle) {
            if let Some(event) = self.event_queue.pop() {
                self.process_event(event);
            }
        }

        // Phase 2: serve stalled T-gates from buffer (FIFO).
        self.try_serve_stalled_gates();

        // Phase 3: schedule newly ready gates.
        self.schedule_ready_gates();
    }

    /// True when every operation (including injected fixups) has completed.
    pub fn is_complete(&self) -> bool {
        self.completed_ops >= self.total_ops
    }

    // ── Trace ID resolution ──────────────────────────────────────────────────

    /// Resolve an arena key to a stable trace ID.
    ///
    /// Original ops → IR `OpId`. Fixup nodes → `SYNTHETIC_ID_FLAG | counter`.
    #[inline]
    fn trace_id(&self, key: OpKey) -> u64 {
        self.key_to_op_id.get(key).copied().unwrap_or(0)
    }

    // ── Event processing ─────────────────────────────────────────────────────

    fn process_event(&mut self, event: TimedEvent) {
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
                // Distinguish fixup completions from regular gate completions.
                if self.dag.get(gate).map(|op| op.kind) == Some(OpKind::Fixup) {
                    self.trace.record(
                        event.cycle,
                        TraceEventKind::FixupCompleted { fixup: gate_id },
                    );
                } else {
                    self.trace
                        .record(event.cycle, TraceEventKind::GateCompleted { gate: gate_id });
                }
                self.completed_ops += 1;
                if !self.hook_table.is_empty() {
                    self.completed_set.insert(gate, ());
                }
                self.complete_gate(gate);
            }
        }
    }

    /// Handle a gate that has finished executing.
    ///
    /// For T-gates and rotations, rolls the injection error die. On error,
    /// inserts a fixup node (already in the ready queue) and increments
    /// `total_ops`. For measurements with hooks, samples the outcome and
    /// activates conditional ops. Otherwise, releases successors.
    fn complete_gate(&mut self, gate: OpKey) {
        let Some(kind) = self.dag.get(gate).map(|op| op.kind) else {
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
            let fixup_key = self.dag.inject_fixup(gate, &mut *self.ready_set);
            let synthetic_id = SYNTHETIC_ID_FLAG | self.next_synthetic_id;
            self.next_synthetic_id += 1;
            self.key_to_op_id.insert(fixup_key, synthetic_id);
            self.trace.record(
                self.current_cycle,
                TraceEventKind::FixupInserted {
                    fixup: synthetic_id,
                    original: gate_id,
                },
            );
            // Fixup is now in ready_set. Count it so is_complete() stays correct.
            self.total_ops += 1;
        } else {
            if let OpKind::Measurement {
                hook: Some(hook_id),
            } = kind
            {
                self.dispatch_hook(gate, hook_id);
            }
            self.dag.release_successors(gate, &mut *self.ready_set);
        }
    }

    /// Sample a measurement outcome and activate the corresponding ops.
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
            &mut *self.ready_set,
        );

        #[allow(clippy::cast_possible_truncation)]
        let activated_count = to_activate.len() as u32;
        self.total_ops += u64::from(activated_count);

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
    fn total_gate_cost(&mut self, gate: OpKey) -> u32 {
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
    ///
    /// T-gates and rotations consume a magic state; if none is available they
    /// are moved to `stalled_gates`. All other gate kinds are scheduled directly.
    fn schedule_ready_gates(&mut self) {
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
    fn schedule_gate_completion(&mut self, gate: OpKey) {
        let cost = self.total_gate_cost(gate);
        self.event_queue.schedule(
            self.current_cycle + u64::from(cost),
            EngineEvent::GateCompleted { gate },
        );
    }

    /// Serve as many stalled gates as the buffer allows (FIFO order).
    ///
    /// Stops as soon as the buffer is empty — once empty, no further gates
    /// in the queue can be served this cycle.
    fn try_serve_stalled_gates(&mut self) {
        // Serve from the front. On buffer exhaustion, break — the front gate and
        // everything behind it stay queued in FIFO order. No scratch allocation.
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
    ///
    /// Called once per factory at construction (via `new`) and once after each
    /// factory event (produced or failed) to keep the factory continuously running.
    fn start_factory(&mut self, factory_id: u16, current_cycle: u64) {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use pirx_hw::{
        CodeType, RoutingConfig,
        model::{
            BufferConfig, FactoryConfig, HardwareModel, InjectionConfig, MetaConfig, QecConfig,
            TimingConfig,
        },
    };
    use pirx_ir::{
        ValidatedCircuit,
        circuit::{CircuitMetadata, OpKind as IrOpKind, Operation, ProfilerCircuit},
    };
    use smallvec::smallvec;

    use super::{Engine, EngineConfig, EngineError};

    fn validated(circuit: ProfilerCircuit) -> ValidatedCircuit {
        pirx_ir::validate::validate(circuit).expect("test fixture must be valid")
    }

    // ── Fixtures ─────────────────────────────────────────────────────────────

    fn cultivation_hw() -> HardwareModel {
        HardwareModel {
            meta: MetaConfig {
                name: "test-cultivation".into(),
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
            factory: FactoryConfig::Cultivation {
                count: 1,
                lambda_raw: 0.002,
                fault_distance: 3,
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

    fn manhattan_hw() -> HardwareModel {
        let mut hw = cultivation_hw();
        hw.routing = RoutingConfig::Manhattan {
            grid_width: 10,
            grid_height: 10,
            cycles_per_hop: 1,
        };
        hw
    }

    fn single_clifford() -> ProfilerCircuit {
        ProfilerCircuit {
            ops: vec![Operation {
                id: 0,
                kind: IrOpKind::Clifford,
                qubits: smallvec![0],
                initially_active: true,
            }],
            deps: vec![],
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: CircuitMetadata {
                name: "smoke".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 1,
                rotation_count: 0,
                depth: 1,
            },
        }
    }

    // ── Construction validation ───────────────────────────────────────────────

    #[test]
    fn rejects_zero_factories() {
        let mut hw = cultivation_hw();
        hw.factory = FactoryConfig::Cultivation {
            count: 0,
            lambda_raw: 0.002,
            fault_distance: 3,
        };
        let circuit = validated(single_clifford());
        assert!(matches!(
            Engine::new(
                &circuit,
                &hw,
                EngineConfig {
                    seed: 0,
                    max_cycles: None
                }
            ),
            Err(EngineError::NoFactories)
        ));
    }

    #[test]
    fn rejects_zero_buffer() {
        let mut hw = cultivation_hw();
        hw.buffer = BufferConfig {
            capacity: 0,
            preload: 0,
        };
        let circuit = validated(single_clifford());
        assert!(matches!(
            Engine::new(
                &circuit,
                &hw,
                EngineConfig {
                    seed: 0,
                    max_cycles: None
                }
            ),
            Err(EngineError::ZeroBuffer)
        ));
    }

    // ── Smoke test ────────────────────────────────────────────────────────────

    /// Single Clifford gate on a cultivation hardware model: engine must
    /// terminate, produce a non-empty trace, and advance at least one cycle.
    #[test]
    fn smoke_single_clifford_cultivation() {
        let circuit = validated(single_clifford());
        let hw = cultivation_hw();
        let config = EngineConfig {
            seed: 42,
            max_cycles: None,
        };

        let engine = Engine::new(&circuit, &hw, config).expect("valid engine");
        let trace = engine.run();

        assert!(
            !trace.events.is_empty(),
            "trace must record at least one event"
        );
        assert!(
            trace.total_cycles > 0,
            "simulation must advance at least one cycle"
        );
    }

    // ── Routing validation ────────────────────────────────────────────────────

    #[test]
    fn rejects_manhattan_without_positions() {
        let hw = manhattan_hw();
        let circuit = validated(single_clifford()); // qubit_positions: None
        assert!(matches!(
            Engine::new(
                &circuit,
                &hw,
                EngineConfig {
                    seed: 0,
                    max_cycles: None
                }
            ),
            Err(EngineError::MissingQubitPositions)
        ));
    }

    #[test]
    fn scalar_routing_does_not_require_positions() {
        let hw = cultivation_hw(); // scalar routing
        let circuit = validated(single_clifford()); // qubit_positions: None
        let engine = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed: 42,
                max_cycles: None,
            },
        );
        assert!(engine.is_ok());
    }
}
