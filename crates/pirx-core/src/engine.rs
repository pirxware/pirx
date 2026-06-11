//! Discrete-event simulation engine.
//!
//! `Engine` drives the circuit DAG through factory events, gate scheduling,
//! magic state consumption, stall tracking, and injection error recovery.
//! All stochastic decisions flow through an explicit `StdRng` seeded from
//! [`EngineConfig::seed`], ensuring full reproducibility (same seed → same trace).

use std::collections::VecDeque;

use pirx_hw::model::HardwareModel;
use pirx_ir::circuit::ProfilerCircuit;
use rand::{Rng as _, SeedableRng, rngs::StdRng};
use slotmap::{Key as _, SecondaryMap};
use thiserror::Error;

use crate::buffer::MagicStateBuffer;
use crate::dag::{Dag, DagError, FifoReadyQueue, OpKey, OpKind, ReadyQueue};
use crate::events::{EngineEvent, EventQueue, TimedEvent};
use crate::factory::{FactoryError, FactoryModel, FactoryOutcome, create_factories};
use crate::trace::{Trace, TraceCollector, TraceEventKind};

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for [`Engine`] construction.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    /// RNG seed. Same seed + same inputs = identical trace, always.
    pub seed: u64,
}

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors that can occur during engine construction.
///
/// All validation happens inside [`Engine::new`]. If `new` returns `Ok`,
/// the simulation is guaranteed to run to completion without error.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("circuit has no operations")]
    EmptyCircuit,

    #[error("dependency graph contains a cycle")]
    CyclicDag,

    #[error("dependency references a non-existent operation")]
    DanglingDependency,

    #[error("too many distinct rotation angles: {0} (maximum 65535)")]
    TooManyRotationAngles(usize),

    #[error("hardware model has zero factories")]
    NoFactories,

    #[error("buffer capacity is zero")]
    ZeroBuffer,

    #[error("Rz synthesis factory is not yet implemented")]
    RzSynthesisNotImplemented,
}

impl From<DagError> for EngineError {
    fn from(err: DagError) -> Self {
        match err {
            DagError::EmptyCircuit => Self::EmptyCircuit,
            DagError::DanglingDependency => Self::DanglingDependency,
            DagError::CyclicDag => Self::CyclicDag,
            DagError::TooManyDistinctAngles(n) => Self::TooManyRotationAngles(n),
        }
    }
}

impl From<FactoryError> for EngineError {
    fn from(err: FactoryError) -> Self {
        match err {
            FactoryError::RzSynthesisNotImplemented => Self::RzSynthesisNotImplemented,
        }
    }
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
    hw: HardwareModel,
    factories: Vec<Box<dyn FactoryModel>>,
    buffer: MagicStateBuffer,

    // Simulation state
    event_queue: EventQueue,
    current_cycle: u64,
    rng: StdRng,
    seed: u64,

    // Stalled T-gates: ready but waiting for a magic state
    stalled_gates: VecDeque<OpKey>,
    /// Cycle at which each gate entered the stall queue, for wait-time accounting.
    stall_start: SecondaryMap<OpKey, u64>,

    // Termination tracking (total_ops grows when fixups are injected)
    total_ops: u64,
    completed_ops: u64,

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
        circuit: &ProfilerCircuit,
        hw: HardwareModel,
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

        // Build DAG — validates circuit structure (acyclicity, dangling deps).
        let dag = Dag::from_circuit(circuit, &hw)?;
        let total_ops = dag.op_count() as u64;

        // Create factory instances from hardware config.
        let factories = create_factories(&hw.factory, &hw.qec)?;
        let factory_count = factories.len();

        // Initialize buffer.
        let buffer = MagicStateBuffer::new(hw.buffer.capacity, hw.buffer.preload);

        // Seed RNG — all stochastic decisions flow through this reference.
        let mut rng = StdRng::seed_from_u64(config.seed);

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

        // Schedule initial factory events and record FactoryStarted at cycle 0.
        let mut event_queue = EventQueue::new();
        for (id, factory) in factories.iter().enumerate() {
            // factory_id: u16 per design — 65 535 max, more than any real architecture.
            #[allow(clippy::cast_possible_truncation)]
            let factory_id = id as u16;
            trace.record(0, TraceEventKind::FactoryStarted { factory_id });
            let outcome = factory.schedule_production(0, &mut rng);
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
            hw,
            factories,
            buffer,
            event_queue,
            current_cycle: 0,
            rng,
            seed: config.seed,
            stalled_gates: VecDeque::new(),
            stall_start: SecondaryMap::new(),
            total_ops,
            completed_ops: 0,
            trace,
        })
    }

    /// Run to completion, returning the sealed [`Trace`].
    ///
    /// Consumes the engine — an engine cannot be run twice.
    pub fn run(mut self) -> Trace {
        while !self.is_complete() {
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
                let gate_raw = gate.data().as_ffi();
                // Distinguish fixup completions from regular gate completions.
                if self.dag.get(gate).map(|op| op.kind) == Some(OpKind::Fixup) {
                    self.trace.record(
                        event.cycle,
                        TraceEventKind::FixupCompleted { fixup: gate_raw },
                    );
                } else {
                    self.trace.record(
                        event.cycle,
                        TraceEventKind::GateCompleted { gate: gate_raw },
                    );
                }
                self.completed_ops += 1;
                self.complete_gate(gate);
            }
        }
    }

    /// Handle a gate that has finished executing.
    ///
    /// For T-gates and rotations, rolls the injection error die. On error,
    /// inserts a fixup node (already in the ready queue) and increments
    /// `total_ops`. Otherwise, releases successors into the ready queue.
    fn complete_gate(&mut self, gate: OpKey) {
        let Some(kind) = self.dag.get(gate).map(|op| op.kind) else {
            return;
        };

        let inject = match kind {
            OpKind::TGate | OpKind::Rotation { .. } => {
                // r#gen: `gen` is a reserved keyword in Rust 2024 edition.
                self.rng.r#gen::<f64>() < self.hw.injection.error_probability
            }
            OpKind::Clifford | OpKind::Measurement | OpKind::Fixup => false,
        };

        if inject {
            let gate_raw = gate.data().as_ffi();
            self.trace.record(
                self.current_cycle,
                TraceEventKind::InjectionError { gate: gate_raw },
            );
            let fixup = self.dag.inject_fixup(gate, &mut *self.ready_set);
            let fixup_raw = fixup.data().as_ffi();
            self.trace.record(
                self.current_cycle,
                TraceEventKind::FixupInserted {
                    fixup: fixup_raw,
                    original: gate_raw,
                },
            );
            // Fixup is now in ready_set. Count it so is_complete() stays correct.
            self.total_ops += 1;
        } else {
            self.dag.release_successors(gate, &mut *self.ready_set);
        }
    }

    /// Schedule all gates currently in the ready queue.
    ///
    /// T-gates and rotations consume a magic state; if none is available they
    /// are moved to `stalled_gates`. All other gate kinds are scheduled directly.
    fn schedule_ready_gates(&mut self) {
        while let Some(gate) = self.ready_set.pop() {
            let gate_raw = gate.data().as_ffi();
            self.trace.record(
                self.current_cycle,
                TraceEventKind::GateReady { gate: gate_raw },
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
                            gate: gate_raw,
                            wait: 0,
                        },
                    );
                    let cost = self.dag.get(gate).map_or(1, |op| op.cycle_cost);
                    self.event_queue.schedule(
                        self.current_cycle + u64::from(cost),
                        EngineEvent::GateCompleted { gate },
                    );
                } else {
                    self.trace.record(
                        self.current_cycle,
                        TraceEventKind::GateStalled { gate: gate_raw },
                    );
                    self.stalled_gates.push_back(gate);
                    self.stall_start.insert(gate, self.current_cycle);
                }
            } else {
                self.trace.record(
                    self.current_cycle,
                    TraceEventKind::GateScheduled { gate: gate_raw },
                );
                let cost = self.dag.get(gate).map_or(1, |op| op.cycle_cost);
                self.event_queue.schedule(
                    self.current_cycle + u64::from(cost),
                    EngineEvent::GateCompleted { gate },
                );
            }
        }
    }

    /// Serve as many stalled gates as the buffer allows (FIFO order).
    ///
    /// Zero-allocation: pops from the front of `stalled_gates`, pushes back
    /// the first unservable gate, and breaks — remaining gates keep their
    /// position in the deque.
    fn try_serve_stalled_gates(&mut self) {
        while let Some(gate) = self.stalled_gates.pop_front() {
            if self.buffer.try_dequeue() {
                let stall_cycle = self.stall_start.remove(gate).unwrap_or(self.current_cycle);
                let wait_u64 = self.current_cycle.saturating_sub(stall_cycle);
                // Stall durations fit in u32 for any realistic simulation;
                // saturate to MAX rather than truncate silently.
                let wait = u32::try_from(wait_u64).unwrap_or(u32::MAX);

                self.trace.record(
                    self.current_cycle,
                    TraceEventKind::BufferDequeue {
                        occupancy: self.buffer.occupancy(),
                    },
                );
                self.trace.record(
                    self.current_cycle,
                    TraceEventKind::GateServed {
                        gate: gate.data().as_ffi(),
                        wait,
                    },
                );
                let cost = self.dag.get(gate).map_or(1, |op| op.cycle_cost);
                self.event_queue.schedule(
                    self.current_cycle + u64::from(cost),
                    EngineEvent::GateCompleted { gate },
                );
            } else {
                // Buffer exhausted — push this gate back and stop.
                // All remaining gates in stalled_gates preserve their FIFO position.
                self.stalled_gates.push_front(gate);
                break;
            }
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
        // Split borrow: factories[idx] and rng are different fields.
        // idx is bounds-checked above, so indexing cannot panic.
        #[allow(clippy::indexing_slicing)]
        let outcome = self.factories[idx].schedule_production(current_cycle, &mut self.rng);
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
    use pirx_hw::model::{BufferConfig, FactoryConfig};

    use super::{Engine, EngineConfig, EngineError};

    // ── Construction validation ───────────────────────────────────────────────

    #[test]
    fn rejects_zero_factories() {
        let mut hw = pirx_testkit::cultivation_hw();
        hw.factory = FactoryConfig::Cultivation {
            count: 0,
            lambda_raw: 0.002,
            fault_distance: 3,
        };
        assert!(matches!(
            Engine::new(
                &pirx_testkit::single_clifford(),
                hw,
                EngineConfig { seed: 0 }
            ),
            Err(EngineError::NoFactories)
        ));
    }

    #[test]
    fn rejects_zero_buffer() {
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer = BufferConfig {
            capacity: 0,
            preload: 0,
        };
        assert!(matches!(
            Engine::new(
                &pirx_testkit::single_clifford(),
                hw,
                EngineConfig { seed: 0 }
            ),
            Err(EngineError::ZeroBuffer)
        ));
    }

    // ── Smoke test ────────────────────────────────────────────────────────────

    /// Single Clifford gate on a cultivation hardware model: engine must
    /// terminate, produce a non-empty trace, and advance at least one cycle.
    #[test]
    fn smoke_single_clifford_cultivation() {
        let engine = Engine::new(
            &pirx_testkit::single_clifford(),
            pirx_testkit::cultivation_hw(),
            EngineConfig { seed: 42 },
        )
        .expect("valid engine");
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
}
