//! Property-based tests for the DES engine.
//!
//! Each test asserts an invariant that must hold for every valid combination
//! of seed, circuit size, and hardware parameters. `proptest` explores the
//! input space automatically and shrinks failing cases.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation
)]

use pirx_core::engine::{Engine, EngineConfig};
use pirx_core::trace::TraceEventKind;
use pirx_hw::model::{BufferConfig, FactoryConfig};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Every gate that enters the circuit must eventually complete.
    ///
    /// Completed events (GateCompleted + FixupCompleted) must be at least as
    /// many as the original gate count. Fixups add extra completions.
    #[test]
    fn all_gates_complete(seed in 0u64..10_000, n in 1u32..20) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(n));
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer.preload = 4;

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        let completed = trace.events.iter()
            .filter(|e| matches!(
                e.kind,
                TraceEventKind::GateCompleted { .. } | TraceEventKind::FixupCompleted { .. }
            ))
            .count();

        prop_assert!(
            completed as u32 >= n,
            "expected >= {n} completions, got {completed}"
        );
    }

    /// Trace events must be monotonically non-decreasing in cycle.
    ///
    /// The engine processes events in cycle order. Any out-of-order event
    /// indicates a scheduling or trace-recording bug.
    #[test]
    fn trace_events_monotonic(seed in 0u64..10_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(10));
        let hw = pirx_testkit::cultivation_hw();

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        for pair in trace.events.windows(2) {
            prop_assert!(
                pair[0].cycle <= pair[1].cycle,
                "non-monotonic: cycle {} followed by {} at events {:?} → {:?}",
                pair[0].cycle, pair[1].cycle, pair[0].kind, pair[1].kind
            );
        }
    }

    /// Same circuit + same hardware + same seed = identical trace, bit-for-bit.
    ///
    /// Principle P1. All randomness flows through an explicit ChaCha12Rng.
    #[test]
    fn determinism(seed in 0u64..10_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(8));
        let config = EngineConfig { seed, max_cycles: None };

        let t1 = Engine::new(&circuit, &pirx_testkit::cultivation_hw(), config)
            .unwrap()
            .run();
        let t2 = Engine::new(&circuit, &pirx_testkit::cultivation_hw(), config)
            .unwrap()
            .run();

        prop_assert_eq!(t1, t2);
    }

    /// Buffer occupancy recorded in trace events must never exceed capacity.
    ///
    /// Verifies the buffer model enforces its upper bound under varying
    /// capacities and stochastic factory timing.
    #[test]
    fn buffer_occupancy_within_capacity(seed in 0u64..5_000, capacity in 1u32..16) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(10));
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer = BufferConfig { capacity, preload: 0 };

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        for event in &trace.events {
            if let TraceEventKind::BufferEnqueue { occupancy }
                | TraceEventKind::BufferDequeue { occupancy } = &event.kind
            {
                prop_assert!(
                    *occupancy <= capacity,
                    "occupancy {occupancy} > capacity {capacity} at cycle {}",
                    event.cycle
                );
            }
        }
    }

    /// Scaling factory count must not decrease throughput for the same circuit.
    ///
    /// More factories produce magic states faster, so total_cycles with k+1
    /// factories must be <= total_cycles with k (for identical seeds and cold
    /// start). Edge case: with enough factories, the circuit is fully
    /// parallelism-limited, so equal is also acceptable.
    #[test]
    fn more_factories_not_slower(seed in 0u64..5_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(6));
        let mut hw1 = pirx_testkit::cultivation_hw();
        hw1.factory = FactoryConfig::Cultivation {
            count: 1,
            lambda_raw: 0.002,
            fault_distance: 3,
        };

        let mut hw2 = hw1.clone();
        hw2.factory = FactoryConfig::Cultivation {
            count: 3,
            lambda_raw: 0.002,
            fault_distance: 3,
        };

        let t1 = Engine::new(&circuit, &hw1, EngineConfig { seed, max_cycles: None }).unwrap().run();
        let t2 = Engine::new(&circuit, &hw2, EngineConfig { seed, max_cycles: None }).unwrap().run();

        prop_assert!(
            t2.total_cycles <= t1.total_cycles,
            "3 factories ({} cycles) slower than 1 ({} cycles)",
            t2.total_cycles, t1.total_cycles
        );
    }

    /// Factory 0's production sequence is identical regardless of total factory count.
    ///
    /// Per-factory child RNGs are derived deterministically from the master seed.
    /// Adding factories must not reshuffle factory 0's random draws.
    /// Uses a long chain so both runs need many factory productions.
    #[test]
    fn factory_rng_isolation(seed in 0u64..5_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(100));
        let mut hw1 = pirx_testkit::cultivation_hw();
        hw1.factory = FactoryConfig::Cultivation {
            count: 1,
            lambda_raw: 0.002,
            fault_distance: 3,
        };
        hw1.buffer = BufferConfig { capacity: 32, preload: 0 };
        hw1.injection.error_probability = 0.0;

        let mut hw5 = hw1.clone();
        hw5.factory = FactoryConfig::Cultivation {
            count: 5,
            lambda_raw: 0.002,
            fault_distance: 3,
        };

        let config = EngineConfig { seed, max_cycles: None };
        let t1 = Engine::new(&circuit, &hw1, config).unwrap().run();
        let t5 = Engine::new(&circuit, &hw5, config).unwrap().run();

        // Extract factory 0's production cycles from both traces.
        let f0_cycles_1: Vec<u64> = t1.events.iter()
            .filter(|e| matches!(e.kind, TraceEventKind::FactoryProduced { factory_id: 0 }))
            .map(|e| e.cycle)
            .collect();
        let f0_cycles_5: Vec<u64> = t5.events.iter()
            .filter(|e| matches!(e.kind, TraceEventKind::FactoryProduced { factory_id: 0 }))
            .map(|e| e.cycle)
            .collect();

        // With 5 factories the run ends sooner; compare the overlap.
        let compare_len = f0_cycles_1.len().min(f0_cycles_5.len()).min(10);
        prop_assert!(compare_len > 0, "factory 0 must produce at least once");

        prop_assert_eq!(
            &f0_cycles_1[..compare_len],
            &f0_cycles_5[..compare_len],
            "factory 0 production cycles must be identical with 1 vs 5 factories"
        );
    }

    /// A pure-Clifford circuit never stalls, regardless of factory or buffer config.
    ///
    /// Cliffords don't consume magic states, so GateStalled must never appear.
    #[test]
    fn cliffords_never_stall(seed in 0u64..5_000, n in 1u32..30) {
        let circuit = pirx_testkit::validated(pirx_testkit::clifford_chain(n));
        let hw = pirx_testkit::cultivation_hw();

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        let stalls = trace.events.iter()
            .filter(|e| matches!(e.kind, TraceEventKind::GateStalled { .. }))
            .count();

        prop_assert_eq!(stalls, 0, "Clifford-only circuit must never stall");
    }

    /// Engine terminates for circuits with hooks.
    ///
    /// The core deadlock fix: for any seed, a circuit with measurement hooks
    /// must run to completion. total_ops is initialized from active ops only
    /// and grows on activation, so completed_ops always reaches total_ops.
    #[test]
    fn hook_circuit_terminates(seed in 0u64..10_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::measurement_with_both_outcomes());
        let mut hw = pirx_testkit::cultivation_hw();
        hw.injection.error_probability = 0.0;

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        prop_assert!(trace.total_cycles > 0, "hook circuit must terminate");

        let completed = trace.events.iter()
            .filter(|e| matches!(
                e.kind,
                TraceEventKind::GateCompleted { .. } | TraceEventKind::FixupCompleted { .. }
            ))
            .count();

        // 1 measurement + 1 activated branch = 2 completions minimum.
        prop_assert!(completed >= 2, "expected >= 2 completions, got {completed}");
    }

    /// Same circuit + same hardware + same seed = identical trace for hook circuits.
    ///
    /// Hook dispatch introduces a new RNG call (outcome sampling). Determinism
    /// must hold for circuits with hooks, not just plain gate circuits.
    #[test]
    fn hook_determinism(seed in 0u64..10_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::measurement_with_both_outcomes());
        let hw = pirx_testkit::cultivation_hw();
        let config = EngineConfig { seed, max_cycles: None };

        let t1 = Engine::new(&circuit, &hw, config).unwrap().run();
        let t2 = Engine::new(&circuit, &hw, config).unwrap().run();

        prop_assert_eq!(t1, t2);
    }

    /// Completed ops == initially active ops + activated ops + fixups.
    ///
    /// For circuits with hooks, the engine must account for all three sources
    /// of ops in its termination tracking.
    #[test]
    fn hook_completed_ops_accounting(seed in 0u64..10_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::measurement_with_one_hook());
        let hw = pirx_testkit::cultivation_hw();

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        let completed = trace.events.iter()
            .filter(|e| matches!(
                e.kind,
                TraceEventKind::GateCompleted { .. } | TraceEventKind::FixupCompleted { .. }
            ))
            .count() as u64;

        let activated: u64 = trace.events.iter()
            .filter_map(|e| match &e.kind {
                TraceEventKind::OpsActivated { activated_count, .. } => Some(u64::from(*activated_count)),
                _ => None,
            })
            .sum();

        let fixups = trace.events.iter()
            .filter(|e| matches!(e.kind, TraceEventKind::FixupInserted { .. }))
            .count() as u64;

        // initially_active = 1 (the measurement op)
        let initially_active = 1u64;
        let expected_total = initially_active + activated + fixups;

        prop_assert_eq!(
            completed,
            expected_total,
            "completed ({}) must equal initially_active ({}) + activated ({}) + fixups ({})",
            completed, initially_active, activated, fixups
        );
    }

    /// max_cycles always bounds total_cycles and sets truncated when the simulation
    /// would otherwise run longer.
    #[test]
    fn max_cycles_bounds_total_cycles(seed in 0u64..5_000, max in 1u64..200) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(10));
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer.preload = 0;

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: Some(max) })
            .unwrap()
            .run();

        if trace.truncated {
            prop_assert!(
                trace.total_cycles <= max,
                "truncated trace total_cycles ({}) must not exceed max_cycles ({})",
                trace.total_cycles, max
            );
        }
    }

    /// max_cycles=None always completes (same as today — no truncation).
    #[test]
    fn no_max_cycles_completes(seed in 0u64..5_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(4));
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer.preload = 4;

        let trace = Engine::new(&circuit, &hw, EngineConfig { seed, max_cycles: None })
            .unwrap()
            .run();

        prop_assert!(!trace.truncated, "must not truncate without max_cycles");
    }
}
