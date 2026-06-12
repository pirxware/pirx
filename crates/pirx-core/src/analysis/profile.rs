//! Execution profile output types — pure data, serializable, no logic.

use serde::{Deserialize, Serialize};

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
}
