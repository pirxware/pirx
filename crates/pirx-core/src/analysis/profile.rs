//! Execution profile output types — pure data, serializable, no logic.

use serde::{Deserialize, Serialize};

/// Per-bucket classification of the dominant execution bottleneck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckType {
    /// No contention: magic state supply meets demand.
    None,
    /// T-gates are waiting for magic states (buffer empty with pending demand).
    FactoryThroughput,
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
    /// Operation identifier from the execution trace.
    /// Original operations: matches `OpId` from the IR circuit.
    /// Fixup nodes: synthetic ID with bit 63 set ([`SYNTHETIC_ID_FLAG`](crate::trace::SYNTHETIC_ID_FLAG)).
    pub gate_id: u64,
    /// Cycles the gate spent waiting for a magic state.
    pub wait_cycles: u64,
}

/// Time-bucketed execution profile — the primary output of [`ProfileAnalyzer`](super::ProfileAnalyzer).
///
/// All `Vec` fields are indexed by bucket `(cycle / resolution)`.
/// Length is always `(total_cycles / resolution) + 1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProfile {
    /// Cycles per time bucket.
    pub resolution: u64,
    /// Total simulation cycles (from [`Trace::total_cycles`](crate::trace::Trace::total_cycles)).
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
    /// Logical error probability per consumed magic state.
    pub p_logical: f64,
    /// Total magic states consumed during simulation.
    pub magic_states_consumed: u64,
    /// Total accumulated infidelity: `magic_states_consumed × p_logical`.
    pub total_infidelity: f64,
    /// Per-bucket magic states consumed in this bucket.
    pub magic_states_per_bucket: Vec<u64>,
}

impl ExecutionProfile {
    /// Per-bucket cumulative magic states consumed (running sum at bucket boundary).
    ///
    /// Computed on demand from [`magic_states_per_bucket`](Self::magic_states_per_bucket).
    pub fn cumulative_magic_states(&self) -> Vec<u64> {
        let mut running = 0u64;
        self.magic_states_per_bucket
            .iter()
            .map(|&c| {
                running = running.saturating_add(c);
                running
            })
            .collect()
    }

    /// Per-bucket cumulative infidelity (running sum × `p_logical`).
    ///
    /// Computed on demand from [`magic_states_per_bucket`](Self::magic_states_per_bucket)
    /// and [`p_logical`](Self::p_logical).
    #[allow(clippy::cast_precision_loss)]
    pub fn cumulative_infidelity(&self) -> Vec<f64> {
        let mut running = 0u64;
        self.magic_states_per_bucket
            .iter()
            .map(|&c| {
                running = running.saturating_add(c);
                running as f64 * self.p_logical
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn profile_with_buckets(buckets: &[u64], p_logical: f64) -> ExecutionProfile {
        let num = buckets.len();
        ExecutionProfile {
            resolution: 10,
            total_cycles: (num as u64) * 10,
            factory_utilization: vec![0.0; num],
            buffer_occupancy: vec![0; num],
            bottleneck_type: vec![BottleneckType::None; num],
            stall_events: Vec::new(),
            injection_errors: 0,
            fixups_inserted: 0,
            critical_path_extension: 0,
            p_logical,
            magic_states_consumed: buckets.iter().sum(),
            total_infidelity: buckets.iter().sum::<u64>() as f64 * p_logical,
            magic_states_per_bucket: buckets.to_vec(),
        }
    }

    #[test]
    fn cumulative_magic_states_is_prefix_sum() {
        let profile = profile_with_buckets(&[3, 0, 2, 5, 1], 0.001);
        assert_eq!(
            profile.cumulative_magic_states(),
            vec![3, 3, 5, 10, 11],
        );
    }

    #[test]
    fn cumulative_magic_states_empty() {
        let profile = profile_with_buckets(&[], 0.001);
        assert!(profile.cumulative_magic_states().is_empty());
    }

    #[test]
    fn cumulative_infidelity_equals_prefix_sum_times_p_logical() {
        let p = 0.002;
        let profile = profile_with_buckets(&[3, 0, 2, 5, 1], p);
        let expected: Vec<f64> = [3, 3, 5, 10, 11]
            .iter()
            .map(|&c| f64::from(c) * p)
            .collect();
        assert_eq!(profile.cumulative_infidelity(), expected);
    }

    #[test]
    fn cumulative_infidelity_zero_p_logical() {
        let profile = profile_with_buckets(&[3, 1, 2], 0.0);
        assert_eq!(
            profile.cumulative_infidelity(),
            vec![0.0, 0.0, 0.0],
        );
    }

    #[test]
    fn cumulative_last_equals_total() {
        let buckets = &[4, 2, 7, 1];
        let p = 0.005;
        let profile = profile_with_buckets(buckets, p);
        let cum = profile.cumulative_magic_states();
        assert_eq!(*cum.last().unwrap(), profile.magic_states_consumed);
        let cum_inf = profile.cumulative_infidelity();
        assert!(
            (cum_inf.last().unwrap() - profile.total_infidelity).abs() < f64::EPSILON,
        );
    }
}
