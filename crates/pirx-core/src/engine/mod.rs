//! Discrete-event simulation engine.
//!
//! `Engine` drives the circuit DAG through factory events, gate scheduling,
//! magic state consumption, stall tracking, and injection error recovery.
//! All stochastic decisions flow through an explicit `ChaCha12Rng` seeded from
//! [`EngineConfig::seed`], ensuring full reproducibility (same seed → same trace).

mod config;
mod dispatch;

use std::collections::VecDeque;

pub use config::{EngineConfig, EngineError};
use dispatch::hook_table_index;
use pirx_hw::{RoutingConfig, model::HardwareModel};
use pirx_ir::ValidatedCircuit;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use slotmap::SecondaryMap;
use smallvec::SmallVec;

use crate::{
    buffer::MagicStateBuffer,
    dag::{Dag, DagBuild, FifoReadyQueue, OpKey},
    events::{EngineEvent, EventQueue},
    factory::{FactoryKind, FactoryModel, FactoryOutcome, create_factories},
    routing,
    trace::{Trace, TraceCollector, TraceEventKind},
};

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
    pub(crate) dag: Dag,
    pub(crate) ready_set: FifoReadyQueue,

    // Hardware state
    pub(crate) injection_error_probability: f64,
    pub(crate) factories: Vec<FactoryKind>,
    pub(crate) buffer: MagicStateBuffer,
    pub(crate) routing: routing::RoutingKind,
    pub(crate) position_index: Vec<(u32, u32)>,

    // Simulation state
    pub(crate) event_queue: EventQueue,
    pub(crate) current_cycle: u64,
    pub(crate) rng: ChaCha12Rng,
    pub(crate) factory_rngs: Vec<ChaCha12Rng>,
    pub(crate) seed: u64,
    pub(crate) max_cycles: Option<u64>,

    // Classical feedback delay in QEC cycles (0 = instant activation).
    pub(crate) feedback_delay: u64,

    // Stalled T-gates: ready but waiting for a magic state.
    pub(crate) stalled_gates: VecDeque<(OpKey, u64)>,

    // Measurement hook dispatch — flat table indexed by hook_id * 2 + outcome.
    pub(crate) hook_table: Vec<SmallVec<[OpKey; 2]>>,
    /// Tracks completed ops for dag.activate_ops() predecessor recomputation.
    pub(crate) completed_set: SecondaryMap<OpKey, ()>,

    // Termination tracking (total_ops grows when fixups are injected or hooks activate)
    pub(crate) total_ops: u64,
    pub(crate) completed_ops: u64,

    // Stable trace IDs: maps arena keys → IR OpId (or synthetic fixup ID).
    pub(crate) key_to_op_id: SecondaryMap<OpKey, u64>,
    pub(crate) next_synthetic_id: u64,

    // Error budget tracking
    /// Logical error probability per consumed magic state (from QecConfig).
    pub(crate) p_logical: f64,
    /// Running count of magic states consumed during simulation.
    pub(crate) magic_states_consumed: u64,

    // Trace collection (append-only, pre-allocated)
    pub(crate) trace: TraceCollector,
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
        let factories = create_factories(&hw.factory, &hw.qec);
        let factory_count = factories.len();

        // Initialize buffer.
        let buffer = MagicStateBuffer::new(hw.buffer.capacity, hw.buffer.preload);

        // Classical feedback delay: ceil(latency_us / cycle_time_us) cycles.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let feedback_delay =
            (hw.timing.classical_feedback_latency_us / hw.timing.cycle_time_us).ceil() as u64;

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
        let mut ready_set = FifoReadyQueue::with_capacity(n_ops);
        for &key in &dag.initial_ready_set() {
            ready_set.push(key);
        }

        // Capacity hint: ~5 events per gate + ~10 per factory (scales with simulated time).
        let capacity_hint = n_ops
            .saturating_mul(5)
            .saturating_add(factory_count.saturating_mul(10));
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
        let mut event_queue = EventQueue::with_capacity(factory_count.saturating_mul(2));
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
            feedback_delay,
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
            p_logical: hw.qec.logical_error_rate(),
            magic_states_consumed: 0,
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
                    return self.trace.finish_truncated(
                        self.seed,
                        self.current_cycle,
                        self.p_logical,
                        self.magic_states_consumed,
                    );
                }
            }
            self.step();
        }
        self.trace.finish(
            self.seed,
            self.current_cycle,
            self.p_logical,
            self.magic_states_consumed,
        )
    }

    /// Advance the simulation by one step (one event-cycle).
    ///
    /// 1. Advance `current_cycle` to the next queued event cycle.
    /// 2. Process every event at that cycle.
    /// 3. Serve stalled T-gates from the buffer (FIFO).
    /// 4. Schedule all newly ready gates.
    pub fn step(&mut self) {
        let Some(cycle) = self.event_queue.peek_cycle() else {
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

    /// Resolve an arena key to a stable trace ID.
    ///
    /// Original ops → IR `OpId`. Fixup nodes → `SYNTHETIC_ID_FLAG | counter`.
    #[inline]
    pub(crate) fn trace_id(&self, key: OpKey) -> u64 {
        self.key_to_op_id.get(key).copied().unwrap_or(0)
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
    use pirx_testkit::{cultivation_hw, single_clifford, validated};

    use super::{Engine, EngineConfig, EngineError};

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
        let hw = pirx_testkit::manhattan_hw(10, 10);
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
            Err(EngineError::MissingQubitPositions)
        ));
    }

    #[test]
    fn scalar_routing_does_not_require_positions() {
        let hw = cultivation_hw();
        let circuit = validated(single_clifford());
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
