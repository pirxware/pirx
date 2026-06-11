//! Integration tests for the DES engine.
//!
//! Hardware models are loaded from TOML fixtures via `include_str!`.
//! Circuits are built by hand from `pirx_ir` types.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pirx_core::{
    engine::{Engine, EngineConfig},
    trace::TraceEventKind,
};
use pirx_hw::{
    CodeType, RoutingConfig,
    model::{
        BufferConfig, DistillationProtocol, FactoryConfig, HardwareModel, InjectionConfig,
        MetaConfig, QecConfig, TimingConfig, load,
    },
};
use pirx_ir::circuit::{CircuitMetadata, Dependency, OpKind, Operation, ProfilerCircuit};
use smallvec::smallvec;

// ── Hardware fixtures ─────────────────────────────────────────────────────────

const CULTIVATION_TOML: &str = include_str!("../../../models/surface_code_d17_cultivation.toml");
const DISTILLATION_TOML: &str = include_str!("../../../models/surface_code_d17_distillation.toml");

fn cultivation_hw() -> HardwareModel {
    load(CULTIVATION_TOML).unwrap()
}

fn distillation_hw() -> HardwareModel {
    load(DISTILLATION_TOML).unwrap()
}

/// Minimal distillation hardware: deterministic 54-cycle production, no aborts.
///
/// `abort_probability = 0.0` so every production round succeeds.
/// `cycles_per_round = 18, rounds = 3` → first magic state at cycle 54, then every 54 cycles.
fn minimal_distillation_hw(
    factory_count: u32,
    buffer_capacity: u32,
    preload: u32,
) -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-minimal".into(),
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

// ── Circuit builders ──────────────────────────────────────────────────────────

fn blank_meta(name: &str) -> CircuitMetadata {
    CircuitMetadata {
        name: name.into(),
        source_framework: "test".into(),
        t_count: 0,
        clifford_count: 0,
        rotation_count: 0,
        depth: 1,
    }
}

fn circuit_clifford() -> ProfilerCircuit {
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
        metadata: blank_meta("clifford"),
    }
}

fn circuit_t_gate() -> ProfilerCircuit {
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
        metadata: blank_meta("t-gate"),
    }
}

fn circuit_two_t_gates() -> ProfilerCircuit {
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
        metadata: blank_meta("two-t-gates"),
    }
}

/// Clifford(0) → TGate(1) → Measurement(2) linear chain.
fn circuit_chain() -> ProfilerCircuit {
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
        metadata: blank_meta("chain"),
    }
}

fn circuit_three_cliffords() -> ProfilerCircuit {
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
                kind: OpKind::Clifford,
                qubits: smallvec![1],
                initially_active: true,
            },
            Operation {
                id: 2,
                kind: OpKind::Clifford,
                qubits: smallvec![2],
                initially_active: true,
            },
        ],
        deps: vec![],
        qubit_count: 3,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("three-cliffords"),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// 1. Single Clifford, no dependencies.
///
/// Cliffords don't consume magic states. The trace must have a GateCompleted
/// event and no GateStalled or GateServed events.
#[test]
fn single_clifford() {
    let trace = Engine::new(
        &circuit_clifford(),
        cultivation_hw(),
        EngineConfig { seed: 0 },
    )
    .unwrap()
    .run();

    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateCompleted { .. })),
        "Clifford must produce a GateCompleted event"
    );
    assert!(
        !trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateStalled { .. })),
        "Clifford must never stall — it consumes no magic state"
    );
    assert!(
        !trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateServed { .. })),
        "Clifford must never appear as GateServed — it has no magic state dependency"
    );
    assert!(
        trace.total_cycles > 0,
        "simulation must advance at least one cycle"
    );
}

/// 2. Single T-gate with buffer pre-loaded to 1 — served immediately, wait=0.
#[test]
fn single_t_gate_served_immediately() {
    let mut hw = cultivation_hw();
    hw.buffer.preload = 1;

    let trace = Engine::new(&circuit_t_gate(), hw, EngineConfig { seed: 0 })
        .unwrap()
        .run();

    let served: Vec<_> = trace
        .events
        .iter()
        .filter(|e| matches!(e.kind, TraceEventKind::GateServed { .. }))
        .collect();
    assert_eq!(served.len(), 1, "exactly one T-gate served");
    assert!(
        matches!(served[0].kind, TraceEventKind::GateServed { wait: 0, .. }),
        "T-gate must be served with wait=0 when the buffer was pre-loaded"
    );
}

/// 3. T-gate stalls when buffer is empty, then served after factory produces.
///
/// Two T-gates, 1 factory (54-cycle production), buffer capacity=1, preload=0.
/// Phase 3 at cycle 54: first T-gate takes the produced state (wait=0), second
/// finds the buffer empty and stalls. Next factory production at cycle 108 serves
/// the stalled gate with wait=54.
#[test]
fn t_gate_stalls_then_served() {
    let hw = minimal_distillation_hw(1, 1, 0);

    let trace = Engine::new(&circuit_two_t_gates(), hw, EngineConfig { seed: 0 })
        .unwrap()
        .run();

    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateStalled { .. })),
        "second T-gate must stall when the single produced state was already consumed"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateServed { wait, .. } if wait > 0)),
        "stalled T-gate must eventually be served with wait > 0"
    );
}

/// 4. Clifford → TGate → Measurement chain respects dependency ordering.
///
/// Each gate becomes ready only after its predecessor completes, so GateCompleted
/// cycles must be strictly increasing: Clifford < TGate < Measurement.
#[test]
fn chain_respects_dependencies() {
    let mut hw = cultivation_hw();
    // Disable injection errors so the chain produces exactly 3 completions.
    hw.injection.error_probability = 0.0;
    // Pre-load the buffer so the T-gate is served without waiting on factory timing.
    hw.buffer.preload = 1;

    let trace = Engine::new(&circuit_chain(), hw, EngineConfig { seed: 0 })
        .unwrap()
        .run();

    let completed_cycles: Vec<u64> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateCompleted { .. } => Some(e.cycle),
            _ => None,
        })
        .collect();

    assert_eq!(
        completed_cycles.len(),
        3,
        "chain with no injection must produce exactly 3 GateCompleted events"
    );
    // Each gate in the chain takes >= 1 cycle, so completions are strictly increasing.
    assert!(
        completed_cycles[0] < completed_cycles[1],
        "Clifford must complete before T-gate: {} < {}",
        completed_cycles[0],
        completed_cycles[1]
    );
    assert!(
        completed_cycles[1] < completed_cycles[2],
        "T-gate must complete before Measurement: {} < {}",
        completed_cycles[1],
        completed_cycles[2]
    );
}

/// 5. Three independent Cliffords — all enter the ready queue in the same engine step.
///
/// Gates with no predecessors are placed in the initial ready set at construction.
/// They are all drained from the queue in phase 3 of the same step, so all three
/// GateReady events must share the same cycle.
#[test]
fn parallel_cliffords() {
    let trace = Engine::new(
        &circuit_three_cliffords(),
        cultivation_hw(),
        EngineConfig { seed: 0 },
    )
    .unwrap()
    .run();

    let ready_cycles: Vec<u64> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateReady { .. } => Some(e.cycle),
            _ => None,
        })
        .collect();

    assert_eq!(
        ready_cycles.len(),
        3,
        "all three Cliffords must emit GateReady"
    );
    let first = ready_cycles[0];
    assert!(
        ready_cycles.iter().all(|&c| c == first),
        "independent Cliffords must become ready in the same engine step (same cycle), \
         got cycles: {ready_cycles:?}"
    );

    let completed = trace
        .events
        .iter()
        .filter(|e| matches!(e.kind, TraceEventKind::GateCompleted { .. }))
        .count();
    assert_eq!(completed, 3, "all three Cliffords must complete");
}

/// 6. Determinism: identical seed + circuit + hardware → identical trace.
#[test]
fn determinism() {
    let circuit = circuit_chain();
    let config = EngineConfig { seed: 42 };

    let t1 = Engine::new(&circuit, cultivation_hw(), config)
        .unwrap()
        .run();
    let t2 = Engine::new(&circuit, cultivation_hw(), config)
        .unwrap()
        .run();
    assert_eq!(
        t1, t2,
        "cultivation: same seed must produce an identical trace"
    );

    let t3 = Engine::new(&circuit, distillation_hw(), config)
        .unwrap()
        .run();
    let t4 = Engine::new(&circuit, distillation_hw(), config)
        .unwrap()
        .run();
    assert_eq!(
        t3, t4,
        "distillation: same seed must produce an identical trace"
    );
}

/// 7. Injection error extends the trace with InjectionError + FixupInserted + FixupCompleted.
///
/// error_probability = 0.5, so roughly half of seeds trigger injection on a T-gate.
/// We scan the first 200 seeds to find a deterministic one that does.
#[test]
fn injection_fixup_extends_trace() {
    let circuit = circuit_t_gate();

    let trace = (0u64..200)
        .find_map(|seed| {
            let mut hw = cultivation_hw();
            hw.buffer.preload = 1; // T-gate served immediately so injection can fire
            let t = Engine::new(&circuit, hw, EngineConfig { seed })
                .unwrap()
                .run();
            if t.events
                .iter()
                .any(|e| matches!(e.kind, TraceEventKind::InjectionError { .. }))
            {
                Some(t)
            } else {
                None
            }
        })
        .expect("at least one seed in 0..200 must trigger injection (error_probability = 0.5)");

    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::InjectionError { .. })),
        "InjectionError must appear when injection fires"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::FixupInserted { .. })),
        "FixupInserted must follow InjectionError"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::FixupCompleted { .. })),
        "FixupCompleted must appear after the fixup op executes"
    );
}
