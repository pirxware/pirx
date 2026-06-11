//! Integration tests for the DES engine.
//!
//! TOML-loaded hardware models exercise the full parsing pipeline.
//! Circuits and manual hardware variants come from `pirx_testkit`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use pirx_core::engine::{Engine, EngineConfig};
use pirx_core::trace::TraceEventKind;
use pirx_hw::model::load;

// ── Hardware fixtures ─────────────────────────────────────────────────────────

const CULTIVATION_TOML: &str = include_str!("../../../models/surface_code_d17_cultivation.toml");
const DISTILLATION_TOML: &str = include_str!("../../../models/surface_code_d17_distillation.toml");

fn cultivation_hw() -> pirx_hw::model::HardwareModel {
    load(CULTIVATION_TOML).unwrap()
}

fn distillation_hw() -> pirx_hw::model::HardwareModel {
    load(DISTILLATION_TOML).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// 1. Single Clifford, no dependencies.
///
/// Cliffords don't consume magic states. The trace must have a GateCompleted
/// event and no GateStalled or GateServed events.
#[test]
fn single_clifford() {
    let trace = Engine::new(
        &pirx_testkit::single_clifford(),
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

    let trace = Engine::new(&pirx_testkit::single_t_gate(), hw, EngineConfig { seed: 0 })
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
    let hw = pirx_testkit::deterministic_distillation_hw(1, 1, 0);

    let trace = Engine::new(
        &pirx_testkit::two_parallel_t_gates(),
        hw,
        EngineConfig { seed: 0 },
    )
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

    let trace = Engine::new(
        &pirx_testkit::clifford_t_measurement_chain(),
        hw,
        EngineConfig { seed: 0 },
    )
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
        &pirx_testkit::parallel_cliffords(3),
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
    let circuit = pirx_testkit::clifford_t_measurement_chain();
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
    let circuit = pirx_testkit::single_t_gate();

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
