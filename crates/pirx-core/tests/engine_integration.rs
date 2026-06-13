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
    trace::{SYNTHETIC_ID_FLAG, TraceEventKind},
};
use pirx_hw::model::{HardwareModel, TimingConfig, load};
use pirx_ir::circuit::{
    ConditionalActivation, Dependency, MeasurementHook, MeasurementOutcome, OpKind, Operation,
    ProfilerCircuit,
};
use pirx_testkit::{
    blank_meta, clifford_t_measurement_chain, cultivation_hw, deterministic_distillation_hw,
    rz_synthesis_hw, single_rotation, single_t_gate, two_parallel_t_gates, validated,
};
use smallvec::smallvec;

// ── Hardware fixtures ─────────────────────────────────────────────────────────

const CULTIVATION_TOML: &str = include_str!("../../../models/surface_code_d17_cultivation.toml");
const DISTILLATION_TOML: &str = include_str!("../../../models/surface_code_d17_distillation.toml");
const RZ_SYNTHESIS_TOML: &str = include_str!("../../../models/surface_code_d17_rz_synthesis.toml");

fn cultivation_hw_toml() -> HardwareModel {
    load(CULTIVATION_TOML).unwrap()
}

fn distillation_hw_toml() -> HardwareModel {
    load(DISTILLATION_TOML).unwrap()
}

fn rz_synthesis_hw_toml() -> HardwareModel {
    load(RZ_SYNTHESIS_TOML).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// 1. Single Clifford, no dependencies.
///
/// Cliffords don't consume magic states. The trace must have a GateCompleted
/// event and no GateStalled or GateServed events.
#[test]
fn single_clifford() {
    let trace = Engine::new(
        &validated(pirx_testkit::single_clifford()),
        &cultivation_hw(),
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
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

    let trace = Engine::new(
        &validated(single_t_gate()),
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
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
    let hw = deterministic_distillation_hw(1, 1, 0);

    let trace = Engine::new(
        &validated(two_parallel_t_gates()),
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
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
        &validated(clifford_t_measurement_chain()),
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
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
        &validated(pirx_testkit::parallel_cliffords(3)),
        &cultivation_hw(),
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
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
    let circuit = validated(clifford_t_measurement_chain());
    let config = EngineConfig {
        seed: 42,
        max_cycles: None,
    };

    let t1 = Engine::new(&circuit, &cultivation_hw_toml(), config)
        .unwrap()
        .run();
    let t2 = Engine::new(&circuit, &cultivation_hw_toml(), config)
        .unwrap()
        .run();
    assert_eq!(
        t1, t2,
        "cultivation: same seed must produce an identical trace"
    );

    let t3 = Engine::new(&circuit, &distillation_hw_toml(), config)
        .unwrap()
        .run();
    let t4 = Engine::new(&circuit, &distillation_hw_toml(), config)
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
    let circuit = validated(single_t_gate());

    let trace = (0u64..200)
        .find_map(|seed| {
            let mut hw = cultivation_hw();
            hw.buffer.preload = 1; // T-gate served immediately so injection can fire
            let t = Engine::new(
                &circuit,
                &hw,
                EngineConfig {
                    seed,
                    max_cycles: None,
                },
            )
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

// ── Hook tests ───────────────────────────────────────────────────────────────

/// 8. Measurement hook circuit terminates for every seed.
///
/// This is the core deadlock fix: circuits with hooks must complete.
/// The engine must activate inactive ops on measurement completion and
/// adjust total_ops so the termination condition is reachable.
#[test]
fn hook_circuit_terminates() {
    let circuit = validated(pirx_testkit::measurement_with_one_hook());
    let mut hw = cultivation_hw();
    hw.injection.error_probability = 0.0;

    for seed in 0u64..100 {
        let trace = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed,
                max_cycles: None,
            },
        )
        .unwrap()
        .run();

        assert!(
            trace.total_cycles > 0,
            "hook circuit must terminate (seed {seed})"
        );
        assert!(
            trace
                .events
                .iter()
                .any(|e| matches!(e.kind, TraceEventKind::GateCompleted { .. })),
            "measurement must complete (seed {seed})"
        );
    }
}

/// 9. Both-outcomes hook: Zero activates op 1, One activates op 2.
///
/// Over enough seeds, both outcomes must appear. For each run, exactly one
/// branch activates (2 completions total: measurement + one branch op).
#[test]
fn hook_both_outcomes_covered() {
    let circuit = validated(pirx_testkit::measurement_with_both_outcomes());
    let mut hw = cultivation_hw();
    hw.injection.error_probability = 0.0;

    let mut saw_zero = false;
    let mut saw_one = false;

    for seed in 0u64..200 {
        let trace = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed,
                max_cycles: None,
            },
        )
        .unwrap()
        .run();

        let outcomes: Vec<_> = trace
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                TraceEventKind::MeasurementOutcome { outcome, .. } => Some(*outcome),
                _ => None,
            })
            .collect();

        assert_eq!(
            outcomes.len(),
            1,
            "exactly one measurement outcome per run (seed {seed})"
        );

        match outcomes[0] {
            MeasurementOutcome::Zero => saw_zero = true,
            MeasurementOutcome::One => saw_one = true,
        }

        // Exactly 2 completions: measurement + one activated branch.
        let completed = trace
            .events
            .iter()
            .filter(|e| matches!(e.kind, TraceEventKind::GateCompleted { .. }))
            .count();
        assert_eq!(
            completed, 2,
            "measurement + exactly one branch must complete (seed {seed})"
        );

        if saw_zero && saw_one {
            break;
        }
    }

    assert!(saw_zero, "Zero outcome must appear in 200 seeds");
    assert!(saw_one, "One outcome must appear in 200 seeds");
}

/// 10. max_cycles truncates the simulation before all ops complete.
///
/// A T-gate chain with no buffer preload takes many cycles (factory must
/// produce states). With max_cycles=10, the engine stops before the first
/// factory production (at cycle 54). total_cycles reflects the last
/// processed cycle, which must be strictly below max_cycles.
#[test]
fn max_cycles_truncates() {
    let hw = deterministic_distillation_hw(1, 1, 0);
    let circuit = validated(two_parallel_t_gates());
    let config = EngineConfig {
        seed: 0,
        max_cycles: Some(10),
    };

    let trace = Engine::new(&circuit, &hw, config).unwrap().run();

    assert!(
        trace.truncated,
        "trace must be truncated when max_cycles is hit"
    );
    assert!(
        trace.total_cycles < 10,
        "total_cycles ({}) must be below max_cycles (10) — the engine stops before \
         processing events at the limit cycle",
        trace.total_cycles
    );
    // Verify the uncapped run would take longer.
    let full = Engine::new(
        &validated(two_parallel_t_gates()),
        &deterministic_distillation_hw(1, 1, 0),
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();
    assert!(!full.truncated, "uncapped run must complete normally");
    assert!(
        full.total_cycles > 10,
        "uncapped run ({} cycles) must exceed the max_cycles limit",
        full.total_cycles
    );
}

/// 11. max_cycles=None runs to completion (same as before).
#[test]
fn max_cycles_none_completes() {
    let hw = deterministic_distillation_hw(1, 1, 1);
    let circuit = validated(single_t_gate());
    let config = EngineConfig {
        seed: 0,
        max_cycles: None,
    };

    let trace = Engine::new(&circuit, &hw, config).unwrap().run();

    assert!(
        !trace.truncated,
        "trace must not be truncated without max_cycles"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateCompleted { .. })),
        "gate must complete when max_cycles is None"
    );
}

/// 12. max_cycles larger than actual simulation length does not truncate.
#[test]
fn max_cycles_larger_than_needed() {
    let mut hw = cultivation_hw();
    hw.injection.error_probability = 0.0;
    let circuit = validated(pirx_testkit::single_clifford());
    let config = EngineConfig {
        seed: 0,
        max_cycles: Some(1_000_000),
    };

    let trace = Engine::new(&circuit, &hw, config).unwrap().run();

    assert!(
        !trace.truncated,
        "trace must not be truncated when max_cycles exceeds actual simulation length"
    );
}

/// 13. Hook-activated T-gate can trigger injection error + fixup.
///
/// Verifies the interaction: measurement → hook activates T-gate → T-gate
/// may trigger injection error → fixup inserted and completed.
#[test]
fn hook_activates_t_gate_with_injection() {
    let circuit = validated(pirx_testkit::hook_activates_t_gate());

    // Find a seed where: outcome=One (T-gate activated) AND injection fires.
    let trace = (0u64..500)
        .find_map(|seed| {
            let mut hw = cultivation_hw();
            hw.buffer.preload = 1;
            let t = Engine::new(
                &circuit,
                &hw,
                EngineConfig {
                    seed,
                    max_cycles: None,
                },
            )
            .unwrap()
            .run();

            let has_activation = t
                .events
                .iter()
                .any(|e| matches!(e.kind, TraceEventKind::OpsActivated { .. }));
            let has_injection = t
                .events
                .iter()
                .any(|e| matches!(e.kind, TraceEventKind::InjectionError { .. }));

            if has_activation && has_injection {
                Some(t)
            } else {
                None
            }
        })
        .expect("at least one seed in 0..500 must trigger hook activation + injection");

    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::FixupInserted { .. })),
        "FixupInserted must follow injection on hook-activated T-gate"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::FixupCompleted { .. })),
        "FixupCompleted must appear after fixup executes"
    );
}

// ── Trace ID correlation tests ──────────────────────────────────────────────

/// 14. GateCompleted events carry the original IR OpIds, not slotmap-internal keys.
///
/// Build a circuit with known OpIds (100, 200, 300). Every GateCompleted
/// event must carry one of those exact values.
#[test]
fn trace_ids_match_circuit_op_ids() {
    let circuit = validated(ProfilerCircuit {
        ops: vec![
            Operation {
                id: 100,
                kind: OpKind::Clifford,
                qubits: smallvec![0],
                initially_active: true,
            },
            Operation {
                id: 200,
                kind: OpKind::Clifford,
                qubits: smallvec![1],
                initially_active: true,
            },
            Operation {
                id: 300,
                kind: OpKind::Clifford,
                qubits: smallvec![2],
                initially_active: true,
            },
        ],
        deps: vec![],
        qubit_count: 3,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("known-ids"),
    });

    let mut hw = cultivation_hw();
    hw.injection.error_probability = 0.0;

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();

    let expected_ids: std::collections::HashSet<u64> = [100, 200, 300].into_iter().collect();

    let completed_ids: Vec<u64> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateCompleted { gate } => Some(gate),
            _ => None,
        })
        .collect();

    assert_eq!(
        completed_ids.len(),
        3,
        "exactly 3 GateCompleted events expected"
    );
    for id in &completed_ids {
        assert!(
            expected_ids.contains(id),
            "GateCompleted carried id={id}, expected one of {expected_ids:?}"
        );
    }
}

/// 15. Fixup nodes carry synthetic IDs with SYNTHETIC_ID_FLAG set.
///
/// Run with error_probability=1.0 so every T-gate triggers injection.
/// FixupInserted.fixup must have bit 63 set, and .original must not.
#[test]
fn fixup_ids_have_synthetic_flag() {
    let circuit = validated(ProfilerCircuit {
        ops: vec![Operation {
            id: 42,
            kind: OpKind::TGate,
            qubits: smallvec![0],
            initially_active: true,
        }],
        deps: vec![],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("fixup-flag"),
    });

    let mut hw = cultivation_hw();
    hw.injection.error_probability = 1.0;
    hw.buffer.preload = 4;

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: Some(10_000),
        },
    )
    .unwrap()
    .run();

    let fixup_events: Vec<_> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::FixupInserted { fixup, original } => Some((fixup, original)),
            _ => None,
        })
        .collect();

    assert!(
        !fixup_events.is_empty(),
        "error_probability=1.0 must produce at least one FixupInserted"
    );

    for (fixup, original) in &fixup_events {
        assert!(
            fixup & SYNTHETIC_ID_FLAG != 0,
            "fixup ID {fixup:#x} must have SYNTHETIC_ID_FLAG set"
        );
        assert!(
            original & SYNTHETIC_ID_FLAG == 0,
            "original ID {original:#x} must NOT have SYNTHETIC_ID_FLAG set"
        );
    }
}

/// 16. All gate lifecycle events (Ready, Scheduled, Completed) carry consistent IDs.
///
/// For a single Clifford with a known OpId, the same ID must appear across
/// all lifecycle events for that gate.
#[test]
fn gate_lifecycle_ids_consistent() {
    let circuit = validated(ProfilerCircuit {
        ops: vec![Operation {
            id: 777,
            kind: OpKind::Clifford,
            qubits: smallvec![0],
            initially_active: true,
        }],
        deps: vec![],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("lifecycle-ids"),
    });

    let mut hw = cultivation_hw();
    hw.injection.error_probability = 0.0;

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();

    let ready_ids: Vec<u64> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateReady { gate } => Some(gate),
            _ => None,
        })
        .collect();
    let scheduled_ids: Vec<u64> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateScheduled { gate } => Some(gate),
            _ => None,
        })
        .collect();
    let completed_ids: Vec<u64> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateCompleted { gate } => Some(gate),
            _ => None,
        })
        .collect();

    assert_eq!(ready_ids, [777], "GateReady must carry OpId 777");
    assert_eq!(scheduled_ids, [777], "GateScheduled must carry OpId 777");
    assert_eq!(completed_ids, [777], "GateCompleted must carry OpId 777");
}

// ── Timing parameter tests ──────────────────────────────────────────────────

/// 17. Measurement gate takes ceil(measurement_time_us / cycle_time_us) cycles.
///
/// With measurement_time_us=3.0, cycle_time_us=1.0, the measurement gate
/// costs 3 QEC cycles. A chain Clifford(0) → Measurement(1) verifies that
/// the measurement completes 3 cycles after the Clifford.
#[test]
fn measurement_timing_from_hardware_model() {
    let mut hw = cultivation_hw();
    hw.timing = TimingConfig {
        cycle_time_us: 1.0,
        measurement_time_us: 3.0,
        classical_feedback_latency_us: 0.0,
    };
    hw.injection.error_probability = 0.0;

    let circuit = validated(ProfilerCircuit {
        ops: vec![
            Operation {
                id: 0,
                kind: OpKind::Clifford,
                qubits: smallvec![0],
                initially_active: true,
            },
            Operation {
                id: 1,
                kind: OpKind::Measurement { hook: None },
                qubits: smallvec![0],
                initially_active: true,
            },
        ],
        deps: vec![Dependency { from: 0, to: 1 }],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("measurement-timing"),
    });

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();

    let completed: Vec<(u64, u64)> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateCompleted { gate } => Some((e.cycle, gate)),
            _ => None,
        })
        .collect();

    assert_eq!(completed.len(), 2, "Clifford + Measurement must complete");
    let clifford_cycle = completed[0].0;
    let measurement_cycle = completed[1].0;
    assert_eq!(
        measurement_cycle - clifford_cycle,
        3,
        "measurement must take 3 cycles (measurement_time_us=3.0, cycle_time_us=1.0)"
    );
}

/// 18. Hook activation deferred by classical_feedback_latency_us.
///
/// With classical_feedback_latency_us=5.0, cycle_time_us=1.0, activated ops
/// don't start until 5 cycles after the measurement completes.
#[test]
fn hook_deferred_by_feedback_latency() {
    let mut hw = cultivation_hw();
    hw.timing = TimingConfig {
        cycle_time_us: 1.0,
        measurement_time_us: 1.0,
        classical_feedback_latency_us: 5.0,
    };
    hw.injection.error_probability = 0.0;

    let circuit = validated(ProfilerCircuit {
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
        metadata: blank_meta("feedback-delay"),
    });

    for seed in 0u64..100 {
        let trace = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed,
                max_cycles: None,
            },
        )
        .unwrap()
        .run();

        let measurement_completed_cycle = trace
            .events
            .iter()
            .find_map(|e| match e.kind {
                TraceEventKind::GateCompleted { gate: 0 } => Some(e.cycle),
                _ => None,
            })
            .expect("measurement must complete");

        let activated_cycle = trace
            .events
            .iter()
            .find_map(|e| match e.kind {
                TraceEventKind::OpsActivated { .. } => Some(e.cycle),
                _ => None,
            })
            .expect("hook must activate ops");

        assert_eq!(
            activated_cycle - measurement_completed_cycle,
            5,
            "activated ops must start 5 cycles after measurement completes (seed {seed})"
        );
    }
}

/// 19. Default timing (measurement_time_us=0.5, cycle_time_us=1.0) produces
///     ceil(0.5) = 1 cycle for measurement gates — no behavior change.
#[test]
fn default_timing_measurement_one_cycle() {
    let mut hw = cultivation_hw();
    hw.injection.error_probability = 0.0;

    let circuit = validated(ProfilerCircuit {
        ops: vec![
            Operation {
                id: 0,
                kind: OpKind::Clifford,
                qubits: smallvec![0],
                initially_active: true,
            },
            Operation {
                id: 1,
                kind: OpKind::Measurement { hook: None },
                qubits: smallvec![0],
                initially_active: true,
            },
        ],
        deps: vec![Dependency { from: 0, to: 1 }],
        qubit_count: 1,
        qubit_positions: None,
        hooks: vec![],
        metadata: blank_meta("default-timing"),
    });

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();

    let completed: Vec<(u64, u64)> = trace
        .events
        .iter()
        .filter_map(|e| match e.kind {
            TraceEventKind::GateCompleted { gate } => Some((e.cycle, gate)),
            _ => None,
        })
        .collect();

    assert_eq!(completed.len(), 2);
    let clifford_cycle = completed[0].0;
    let measurement_cycle = completed[1].0;
    assert_eq!(
        measurement_cycle - clifford_cycle,
        1,
        "default timing: measurement must take 1 cycle (ceil(0.5/1.0) = 1)"
    );
}

// ── Rz synthesis tests ──────────────────────────────────────────────────────

/// 20. Rz synthesis factory produces rotation states; circuit runs to completion.
#[test]
fn rz_synthesis_runs_to_completion() {
    let circuit = validated(single_rotation());
    let hw = rz_synthesis_hw();

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 42,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();

    assert!(
        trace.total_cycles > 0,
        "Rz synthesis circuit must run to completion"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateCompleted { .. })),
        "rotation gate must complete"
    );
}

/// 21. Rz synthesis is deterministic: same seed → same trace.
#[test]
fn rz_synthesis_deterministic() {
    let circuit = validated(single_rotation());
    let config = EngineConfig {
        seed: 99,
        max_cycles: None,
    };

    let t1 = Engine::new(&circuit, &rz_synthesis_hw(), config)
        .unwrap()
        .run();
    let t2 = Engine::new(&circuit, &rz_synthesis_hw(), config)
        .unwrap()
        .run();
    assert_eq!(
        t1, t2,
        "rz_synthesis: same seed must produce an identical trace"
    );
}

/// 22. Rotation gate consumes a magic state from the buffer.
#[test]
fn rotation_consumes_magic_state() {
    let mut hw = rz_synthesis_hw();
    hw.buffer.preload = 1;
    hw.injection.error_probability = 0.0;

    let circuit = validated(single_rotation());

    let trace = Engine::new(
        &circuit,
        &hw,
        EngineConfig {
            seed: 0,
            max_cycles: None,
        },
    )
    .unwrap()
    .run();

    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::BufferDequeue { .. })),
        "rotation gate must trigger a BufferDequeue event"
    );
    assert!(
        trace
            .events
            .iter()
            .any(|e| matches!(e.kind, TraceEventKind::GateServed { .. })),
        "rotation gate must appear as GateServed (magic state consumed)"
    );
}

/// 23. Rz synthesis TOML model file loads and validates successfully.
#[test]
fn rz_synthesis_toml_loads() {
    let hw = rz_synthesis_hw_toml();
    assert_eq!(hw.meta.name, "surface_code_d17_rz_synthesis_8fac");
}
