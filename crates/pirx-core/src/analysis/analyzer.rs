//! Post-hoc trace analyzer — single O(n) pass over raw [`TraceEvent`]s.

use super::profile::{BottleneckType, ExecutionProfile, StallRecord};
use crate::trace::{Trace, TraceEventKind};

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
        // Difference array: deltas[b] += 1 at interval start, deltas[b+1] -= 1 at
        // interval end. Prefix-summed into active counts after the event loop.
        let mut factory_active_deltas = vec![0i64; num_buckets.saturating_add(1)];
        let mut factory_failure_counts = vec![0u32; num_buckets];
        let mut buffer_occupancy = vec![0u32; num_buckets];
        let mut stalls_in_bucket = vec![0u32; num_buckets];
        let mut stall_events: Vec<StallRecord> = Vec::new();
        let mut injection_errors: u64 = 0;
        let mut fixups_inserted: u64 = 0;
        let mut critical_path_extension: u64 = 0;
        let mut magic_states_per_bucket = vec![0u64; num_buckets];

        // Per-factory start cycle for active-interval tracking.
        // Dense Vec indexed by factory_id — u16 keyspace, no hash overhead.
        let mut factory_starts: Vec<Option<u64>> = vec![None; usize::from(factory_count)];
        // Per-fixup insert cycle for critical-path extension accounting.
        let mut fixup_starts: std::collections::HashMap<u64, u64> =
            std::collections::HashMap::new();

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
                    if let Some(slot) = factory_starts.get_mut(usize::from(*factory_id)) {
                        *slot = Some(event.cycle);
                    }
                }

                TraceEventKind::FactoryProduced { factory_id } => {
                    if let Some(start) = factory_starts
                        .get(usize::from(*factory_id))
                        .copied()
                        .flatten()
                    {
                        let start_b = to_bucket(start);
                        if let Some(d) = factory_active_deltas.get_mut(start_b) {
                            *d += 1;
                        }
                        if let Some(d) = factory_active_deltas.get_mut(b + 1) {
                            *d -= 1;
                        }
                    }
                }

                TraceEventKind::FactoryFailed { factory_id } => {
                    if let Some(start) = factory_starts
                        .get(usize::from(*factory_id))
                        .copied()
                        .flatten()
                    {
                        let start_b = to_bucket(start);
                        if let Some(d) = factory_active_deltas.get_mut(start_b) {
                            *d += 1;
                        }
                        if let Some(d) = factory_active_deltas.get_mut(b + 1) {
                            *d -= 1;
                        }
                    }
                    if let Some(c) = factory_failure_counts.get_mut(b) {
                        *c = c.saturating_add(1);
                    }
                }

                TraceEventKind::BufferEnqueue { occupancy }
                | TraceEventKind::BufferDequeue { occupancy } => {
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
                    if let Some(count) = magic_states_per_bucket.get_mut(b) {
                        *count = count.saturating_add(1);
                    }
                }

                TraceEventKind::GateServed { .. } => {
                    if let Some(count) = magic_states_per_bucket.get_mut(b) {
                        *count = count.saturating_add(1);
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

                TraceEventKind::GateReady { .. }
                | TraceEventKind::GateScheduled { .. }
                | TraceEventKind::GateStalled { .. }
                | TraceEventKind::GateCompleted { .. }
                | TraceEventKind::BufferFull
                | TraceEventKind::RoutingStarted { .. }
                | TraceEventKind::RoutingCompleted { .. }
                | TraceEventKind::MeasurementOutcome { .. }
                | TraceEventKind::OpsActivated { .. } => {}
            }
        }

        // Fill remaining partial factory runs up to total_cycles.
        let last_b = to_bucket(total_cycles);
        for &start in factory_starts.iter().flatten() {
            let start_b = to_bucket(start);
            if let Some(d) = factory_active_deltas.get_mut(start_b) {
                *d += 1;
            }
            if let Some(d) = factory_active_deltas.get_mut(last_b + 1) {
                *d -= 1;
            }
        }

        // Prefix-sum the difference array into per-bucket active counts.
        let fcount = f64::from(factory_count.max(1));
        let mut running: i64 = 0;
        let factory_utilization: Vec<f64> = factory_active_deltas
            .get(..num_buckets)
            .unwrap_or_default()
            .iter()
            .map(|&delta| {
                running += delta;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let active = if running > 0 { running as u32 } else { 0 };
                (f64::from(active) / fcount).min(1.0)
            })
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

        // Cumulative magic state consumption and infidelity — single fused pass.
        let p_logical = trace.p_logical;
        let total_magic_states = trace.magic_states_consumed;
        let total_infidelity = total_magic_states as f64 * p_logical;
        let mut cumulative_magic_states = vec![0u64; num_buckets];
        let mut cumulative_infidelity = vec![0.0f64; num_buckets];
        let mut running_ms: u64 = 0;
        for (i, &count) in magic_states_per_bucket.iter().enumerate() {
            running_ms = running_ms.saturating_add(count);
            if let Some(slot) = cumulative_magic_states.get_mut(i) {
                *slot = running_ms;
            }
            if let Some(slot) = cumulative_infidelity.get_mut(i) {
                *slot = running_ms as f64 * p_logical;
            }
        }

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
            p_logical,
            magic_states_consumed: total_magic_states,
            total_infidelity,
            cumulative_magic_states,
            cumulative_infidelity,
            magic_states_per_bucket,
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
    use pirx_hw::model::HardwareModel;
    use pirx_testkit::{cultivation_hw, single_clifford, t_gate_chain, validated};

    use super::{BottleneckType, ProfileAnalyzer};
    use crate::{
        engine::{Engine, EngineConfig},
        trace::TraceEventKind,
    };

    fn cultivation_cold(factory_count: u32) -> HardwareModel {
        let mut hw = cultivation_hw();
        hw.factory = pirx_hw::model::FactoryConfig::Cultivation {
            count: factory_count,
            lambda_raw: 0.002,
            fault_distance: 3,
        };
        hw
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn total_cycles_matches_trace() {
        let circuit = validated(t_gate_chain(5));
        let hw = cultivation_cold(1);
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

        let expected = trace.total_cycles;
        let profile = ProfileAnalyzer::analyze(&trace, 1, 10);

        assert_eq!(profile.total_cycles, expected);
    }

    #[test]
    fn injection_errors_count_matches_trace() {
        let circuit = validated(t_gate_chain(5));
        let hw = cultivation_cold(1);
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

        let trace_count = trace
            .events
            .iter()
            .filter(|e| matches!(e.kind, TraceEventKind::InjectionError { .. }))
            .count() as u64;

        let profile = ProfileAnalyzer::analyze(&trace, 1, 10);
        assert_eq!(profile.injection_errors, trace_count);
    }

    #[test]
    fn factory_utilization_in_range() {
        let circuit = validated(t_gate_chain(5));
        let hw = cultivation_cold(2);
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

    #[test]
    fn stall_events_nonempty_on_cold_start() {
        let circuit = validated(t_gate_chain(5));
        let hw = cultivation_cold(1);
        let trace = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed: 7,
                max_cycles: None,
            },
        )
        .unwrap()
        .run();

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

    #[test]
    fn profile_vector_lengths_consistent() {
        let circuit = validated(t_gate_chain(5));
        let hw = cultivation_cold(1);
        let trace = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed: 1,
                max_cycles: None,
            },
        )
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

    #[test]
    fn no_injection_errors_when_probability_zero() {
        let circuit = validated(t_gate_chain(5));
        let mut hw = cultivation_cold(1);
        hw.injection.error_probability = 0.0;
        hw.buffer.preload = 4;
        let trace = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed: 99,
                max_cycles: None,
            },
        )
        .unwrap()
        .run();

        let profile = ProfileAnalyzer::analyze(&trace, 1, 1);
        assert_eq!(profile.injection_errors, 0);
        assert_eq!(profile.fixups_inserted, 0);
        assert_eq!(profile.critical_path_extension, 0);
    }

    #[test]
    fn bottleneck_none_when_no_stalls() {
        let clifford = validated(single_clifford());
        let hw = cultivation_cold(1);
        let trace = Engine::new(
            &clifford,
            &hw,
            EngineConfig {
                seed: 0,
                max_cycles: None,
            },
        )
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

    #[test]
    fn difference_array_matches_naive_reference() {
        let circuit = validated(t_gate_chain(5));
        let hw = cultivation_cold(2);
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

        let resolution = 1u64;
        let factory_count = 2u16;
        let profile = ProfileAnalyzer::analyze(&trace, factory_count, resolution);

        let num_buckets = usize::try_from(trace.total_cycles / resolution + 1).unwrap();
        let mut naive_active = vec![0u32; num_buckets];
        let mut starts = std::collections::HashMap::<u16, u64>::new();

        let to_b = |c: u64| usize::try_from(c / resolution).unwrap();

        for event in &trace.events {
            let b = to_b(event.cycle);
            match &event.kind {
                TraceEventKind::FactoryStarted { factory_id } => {
                    starts.insert(*factory_id, event.cycle);
                }
                TraceEventKind::FactoryProduced { factory_id }
                | TraceEventKind::FactoryFailed { factory_id } => {
                    if let Some(&s) = starts.get(factory_id) {
                        for slot in naive_active.iter_mut().take(b + 1).skip(to_b(s)) {
                            *slot += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        let last_b = to_b(trace.total_cycles);
        for &s in starts.values() {
            for slot in naive_active.iter_mut().take(last_b + 1).skip(to_b(s)) {
                *slot += 1;
            }
        }

        let fcount = f64::from(factory_count.max(1));
        for (i, (&naive, &optimized)) in naive_active
            .iter()
            .zip(profile.factory_utilization.iter())
            .enumerate()
        {
            let expected = (f64::from(naive) / fcount).min(1.0);
            assert!(
                (expected - optimized).abs() < f64::EPSILON,
                "bucket {i}: naive={expected}, optimized={optimized}"
            );
        }
    }
}
