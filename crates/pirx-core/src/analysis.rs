//! Post-hoc trace analysis — single O(n) pass over raw [`TraceEvent`]s.
//!
//! [`ProfileAnalyzer::analyze`] reads a [`Trace`] produced by the engine and
//! returns a time-bucketed [`ExecutionProfile`]. No engine state is touched;
//! the analyzer is a pure function of the trace.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::trace::{Trace, TraceEventKind};

// ── Output types ──────────────────────────────────────────────────────────────

/// Per-bucket classification of the dominant execution bottleneck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckType {
    /// No contention: magic state supply meets demand.
    None,
    /// T-gates are waiting for magic states (buffer empty with pending demand).
    FactoryThroughput,
    /// Operations are waiting for routing paths.
    /// Placeholder — the scalar routing model produces no routing contention.
    RoutingContention,
    /// Both factory failures and gate stalls occurred in the same bucket.
    Balanced,
}

/// A single stall record: one gate that waited for a magic state.
///
/// Sourced from [`TraceEventKind::GateServed`] events where `wait > 0`.
/// The wait duration is computed by the engine; we consume it directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StallRecord {
    /// Cycle at which the gate was finally served.
    pub cycle: u64,
    /// Raw gate identifier (matches [`pirx_ir::circuit::OpId`]).
    pub gate_id: u64,
    /// Cycles the gate spent waiting for a magic state.
    pub wait_cycles: u64,
}

/// Time-bucketed execution profile — the primary output of [`ProfileAnalyzer`].
///
/// All `Vec` fields are indexed by bucket `(cycle / resolution)`.
/// Length is always `(total_cycles / resolution) + 1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProfile {
    /// Cycles per time bucket.
    pub resolution: u64,
    /// Total simulation cycles (from [`Trace::total_cycles`]).
    pub total_cycles: u64,
    /// Per-bucket fraction of factories in active production. Range: `[0.0, 1.0]`.
    pub factory_utilization: Vec<f64>,
    /// Per-bucket magic-state buffer occupancy (last value observed in bucket).
    pub buffer_occupancy: Vec<u32>,
    /// Per-bucket bottleneck classification.
    pub bottleneck_type: Vec<BottleneckType>,
    /// Individual stall records for all gates that waited for a magic state.
    pub stall_events: Vec<StallRecord>,
    /// Total count of [`TraceEventKind::InjectionError`] events.
    pub injection_errors: u64,
    /// Total count of [`TraceEventKind::FixupInserted`] events.
    pub fixups_inserted: u64,
    /// Sum of fixup durations: total cycles added to the circuit by injection errors.
    pub critical_path_extension: u64,
}

// ── Analyzer ──────────────────────────────────────────────────────────────────

/// Post-hoc trace analyzer. Stateless; every call to [`analyze`] is independent.
///
/// [`analyze`]: ProfileAnalyzer::analyze
pub struct ProfileAnalyzer;

impl ProfileAnalyzer {
    /// Analyze `trace` into an [`ExecutionProfile`].
    ///
    /// `factory_count` normalizes factory utilization to `[0.0, 1.0]`.
    /// `resolution` is cycles per bucket; clamped to `1`.
    ///
    /// Makes a single O(n) pass over `trace.events`. All output vectors are
    /// pre-allocated before the scan loop — zero allocations inside the loop.
    pub fn analyze(trace: &Trace, factory_count: u16, resolution: u64) -> ExecutionProfile {
        let resolution = resolution.max(1);
        let total_cycles = trace.total_cycles;
        // usize::try_from guards against truncation on 32-bit targets; in any
        // realistic simulation total_cycles/resolution fits comfortably in usize.
        let num_buckets = usize::try_from((total_cycles / resolution).saturating_add(1))
            .unwrap_or(usize::MAX / 2);

        // Pre-allocate all accumulation buffers.
        let mut factory_active_counts = vec![0u32; num_buckets];
        let mut factory_failure_counts = vec![0u32; num_buckets];
        let mut buffer_occupancy = vec![0u32; num_buckets];
        let mut stalls_in_bucket = vec![0u32; num_buckets];
        let mut stall_events: Vec<StallRecord> = Vec::new();
        let mut injection_errors: u64 = 0;
        let mut fixups_inserted: u64 = 0;
        let mut critical_path_extension: u64 = 0;

        // Per-factory start cycle for active-interval tracking.
        // Capacity bounded by factory_count.
        let mut factory_starts: HashMap<u16, u64> =
            HashMap::with_capacity(usize::from(factory_count));
        // Per-fixup insert cycle for critical-path extension accounting.
        let mut fixup_starts: HashMap<u64, u64> = HashMap::new();

        let to_bucket = |cycle: u64| -> usize {
            usize::try_from(cycle / resolution)
                .unwrap_or(usize::MAX)
                .min(num_buckets.saturating_sub(1))
        };

        // ── Single pass ───────────────────────────────────────────────────────
        for event in &trace.events {
            let b = to_bucket(event.cycle);
            match &event.kind {
                TraceEventKind::FactoryStarted { factory_id } => {
                    factory_starts.insert(*factory_id, event.cycle);
                }

                TraceEventKind::FactoryProduced { factory_id } => {
                    // Mark every bucket this factory's run spanned as active.
                    // FactoryStarted for the next run is recorded immediately after
                    // in the same cycle, so factory_starts is still the OLD start here.
                    if let Some(&start) = factory_starts.get(factory_id) {
                        let start_b = to_bucket(start);
                        for fill_b in start_b..=b {
                            if let Some(c) = factory_active_counts.get_mut(fill_b) {
                                *c = c.saturating_add(1);
                            }
                        }
                    }
                }

                TraceEventKind::FactoryFailed { factory_id } => {
                    if let Some(&start) = factory_starts.get(factory_id) {
                        let start_b = to_bucket(start);
                        for fill_b in start_b..=b {
                            if let Some(c) = factory_active_counts.get_mut(fill_b) {
                                *c = c.saturating_add(1);
                            }
                        }
                    }
                    if let Some(c) = factory_failure_counts.get_mut(b) {
                        *c = c.saturating_add(1);
                    }
                }

                TraceEventKind::BufferEnqueue { occupancy }
                | TraceEventKind::BufferDequeue { occupancy } => {
                    // Last observed occupancy in the bucket wins.
                    if let Some(occ) = buffer_occupancy.get_mut(b) {
                        *occ = *occupancy;
                    }
                }

                TraceEventKind::GateServed { gate, wait } if *wait > 0 => {
                    stall_events.push(StallRecord {
                        cycle: event.cycle,
                        gate_id: *gate,
                        wait_cycles: u64::from(*wait),
                    });
                    if let Some(c) = stalls_in_bucket.get_mut(b) {
                        *c = c.saturating_add(1);
                    }
                }

                TraceEventKind::InjectionError { .. } => {
                    injection_errors += 1;
                }

                TraceEventKind::FixupInserted { fixup, .. } => {
                    fixups_inserted += 1;
                    fixup_starts.insert(*fixup, event.cycle);
                }

                TraceEventKind::FixupCompleted { fixup } => {
                    if let Some(start) = fixup_starts.remove(fixup) {
                        critical_path_extension = critical_path_extension
                            .saturating_add(event.cycle.saturating_sub(start));
                    }
                }

                // GateReady, GateScheduled, GateStalled, GateCompleted (wait=0 case
                // of GateServed), BufferFull, RoutingStarted, RoutingCompleted,
                // MeasurementOutcome, OpsActivated — no additional metric
                // contribution beyond what is already accumulated.
                _ => {}
            }
        }

        // Fill remaining partial factory runs up to total_cycles.
        // The simulation ends when all gates complete, not when all factories finish.
        // factory_starts now holds the START of each factory's most recent (still
        // in-flight) production run — fill those intervals to the last bucket.
        let last_b = to_bucket(total_cycles);
        for &start in factory_starts.values() {
            let start_b = to_bucket(start);
            for fill_b in start_b..=last_b {
                if let Some(c) = factory_active_counts.get_mut(fill_b) {
                    *c = c.saturating_add(1);
                }
            }
        }

        // Normalize active counts → utilization in [0.0, 1.0].
        // factory_count.max(1) guards against the pathological zero-factory case.
        let fcount = f64::from(factory_count.max(1));
        let factory_utilization: Vec<f64> = factory_active_counts
            .iter()
            .map(|&active| (f64::from(active) / fcount).min(1.0))
            .collect();

        // Classify bottleneck per bucket from stall counts and factory failure counts.
        let bottleneck_type: Vec<BottleneckType> = stalls_in_bucket
            .iter()
            .zip(factory_failure_counts.iter())
            .map(|(&stalls, &failures)| {
                if stalls > 0 && failures > 0 {
                    BottleneckType::Balanced
                } else if stalls > 0 {
                    BottleneckType::FactoryThroughput
                } else {
                    BottleneckType::None
                }
            })
            .collect();

        ExecutionProfile {
            resolution,
            total_cycles,
            factory_utilization,
            buffer_occupancy,
            bottleneck_type,
            stall_events,
            injection_errors,
            fixups_inserted,
            critical_path_extension,
        }
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
    use pirx_ir::circuit::{CircuitMetadata, Dependency, OpKind, Operation, ProfilerCircuit};
    use smallvec::smallvec;

    use crate::{
        engine::{Engine, EngineConfig},
        trace::TraceEventKind,
    };

    use super::{BottleneckType, ProfileAnalyzer};

    // ── Fixtures ──────────────────────────────────────────────────────────────

    /// Single cultivation factory, cold start (preload=0), injection errors enabled.
    fn cultivation_cold(factory_count: u32) -> HardwareModel {
        HardwareModel {
            meta: MetaConfig {
                name: "test-cultivation-cold".into(),
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
                count: factory_count,
                lambda_raw: 0.002,
                fault_distance: 3,
            },
            injection: InjectionConfig {
                error_probability: 0.5,
                fixup_cost_cycles: 1,
            },
            routing: RoutingConfig::default(),
            buffer: BufferConfig {
                capacity: 4,
                preload: 0,
            },
        }
    }

    /// 5 T-gates in a strict linear chain: T0 → T1 → T2 → T3 → T4.
    fn chain_5_t_gates() -> ProfilerCircuit {
        let ops: Vec<Operation> = (0u64..5)
            .map(|id| Operation {
                id,
                kind: OpKind::TGate,
                qubits: smallvec![0],
                initially_active: true,
            })
            .collect();
        let deps: Vec<Dependency> = (0u64..4)
            .map(|i| Dependency { from: i, to: i + 1 })
            .collect();
        ProfilerCircuit {
            ops,
            deps,
            qubit_count: 1,
            qubit_positions: None,
            hooks: vec![],
            metadata: CircuitMetadata {
                name: "chain-5-t".into(),
                source_framework: "test".into(),
                t_count: 5,
                clifford_count: 0,
                rotation_count: 0,
                depth: 5,
            },
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// total_cycles in the profile must match trace.total_cycles exactly.
    #[test]
    fn total_cycles_matches_trace() {
        let circuit = chain_5_t_gates();
        let hw = cultivation_cold(1);
        let trace = Engine::new(&circuit, &hw, EngineConfig { seed: 0 })
            .unwrap()
            .run();

        let expected = trace.total_cycles;
        let profile = ProfileAnalyzer::analyze(&trace, 1, 10);

        assert_eq!(profile.total_cycles, expected);
    }

    /// injection_errors must equal the number of InjectionError trace events.
    #[test]
    fn injection_errors_count_matches_trace() {
        let circuit = chain_5_t_gates();
        let hw = cultivation_cold(1);
        let trace = Engine::new(&circuit, &hw, EngineConfig { seed: 0 })
            .unwrap()
            .run();

        let trace_count = trace
            .events
            .iter()
            .filter(|e| matches!(e.kind, TraceEventKind::InjectionError { .. }))
            .count() as u64;

        let profile = ProfileAnalyzer::analyze(&trace, 1, 10);
        assert_eq!(profile.injection_errors, trace_count);
    }

    /// Every factory_utilization value must be in [0.0, 1.0].
    #[test]
    fn factory_utilization_in_range() {
        let circuit = chain_5_t_gates();
        let hw = cultivation_cold(2);
        let trace = Engine::new(&circuit, &hw, EngineConfig { seed: 42 })
            .unwrap()
            .run();

        let profile = ProfileAnalyzer::analyze(&trace, 2, 5);

        assert!(
            !profile.factory_utilization.is_empty(),
            "profile must have at least one bucket"
        );
        for (i, &u) in profile.factory_utilization.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(&u),
                "bucket {i}: factory_utilization {u} is outside [0.0, 1.0]"
            );
        }
    }

    /// With a cold-start buffer (preload=0) and a cultivation factory, the first
    /// T-gate in the chain cannot be served until the factory completes at least
    /// one production cycle. stall_events must therefore be non-empty.
    #[test]
    fn stall_events_nonempty_on_cold_start() {
        let circuit = chain_5_t_gates();
        let hw = cultivation_cold(1);
        let trace = Engine::new(&circuit, &hw, EngineConfig { seed: 7 })
            .unwrap()
            .run();

        // Confirm the trace itself has stalled-then-served events.
        assert!(
            trace
                .events
                .iter()
                .any(|e| matches!(e.kind, TraceEventKind::GateServed { wait, .. } if wait > 0)),
            "trace must contain at least one GateServed with wait > 0 (cold start)"
        );

        let profile = ProfileAnalyzer::analyze(&trace, 1, 10);
        assert!(
            !profile.stall_events.is_empty(),
            "stall_events must be non-empty when buffer starts cold"
        );
    }

    /// bottleneck_type length must equal factory_utilization length (num_buckets).
    #[test]
    fn profile_vector_lengths_consistent() {
        let circuit = chain_5_t_gates();
        let hw = cultivation_cold(1);
        let trace = Engine::new(&circuit, &hw, EngineConfig { seed: 1 })
            .unwrap()
            .run();

        let resolution = 8;
        let profile = ProfileAnalyzer::analyze(&trace, 1, resolution);
        let expected_buckets = usize::try_from((trace.total_cycles / resolution).saturating_add(1))
            .unwrap_or(usize::MAX / 2);

        assert_eq!(profile.factory_utilization.len(), expected_buckets);
        assert_eq!(profile.buffer_occupancy.len(), expected_buckets);
        assert_eq!(profile.bottleneck_type.len(), expected_buckets);
    }

    /// A simulation with no injection errors must report zero injection_errors
    /// and zero fixups_inserted.
    #[test]
    fn no_injection_errors_when_probability_zero() {
        let circuit = chain_5_t_gates();
        let mut hw = cultivation_cold(1);
        hw.injection.error_probability = 0.0;
        hw.buffer.preload = 4; // warm start so T-gates don't stall
        let trace = Engine::new(&circuit, &hw, EngineConfig { seed: 99 })
            .unwrap()
            .run();

        let profile = ProfileAnalyzer::analyze(&trace, 1, 1);
        assert_eq!(profile.injection_errors, 0);
        assert_eq!(profile.fixups_inserted, 0);
        assert_eq!(profile.critical_path_extension, 0);
    }

    /// A pure-Clifford circuit never needs magic states, so no stalls occur and
    /// bottleneck_type must be None in every bucket.
    #[test]
    fn bottleneck_none_when_no_stalls() {
        let clifford = ProfilerCircuit {
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
            metadata: CircuitMetadata {
                name: "clifford".into(),
                source_framework: "test".into(),
                t_count: 0,
                clifford_count: 1,
                rotation_count: 0,
                depth: 1,
            },
        };
        let hw = cultivation_cold(1);
        let trace = Engine::new(&clifford, &hw, EngineConfig { seed: 0 })
            .unwrap()
            .run();

        let profile = ProfileAnalyzer::analyze(&trace, 1, 1);
        for (i, &bt) in profile.bottleneck_type.iter().enumerate() {
            assert_eq!(
                bt,
                BottleneckType::None,
                "bucket {i}: expected None, got {bt:?} (no stalls expected with warm start)"
            );
        }
    }
}
