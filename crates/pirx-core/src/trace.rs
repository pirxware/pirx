//! Trace event types — collected during simulation, analyzed after.
//!
//! 32 bytes per event (cycle: 8 + kind: 24). Append-only during
//! simulation, immutable after. Analyzed by the profile analyzer.

use serde::{Deserialize, Serialize};

// ── Size budget ──────────────────────────────────────────────────────────────
//
// TraceEvent must stay at 32 bytes (cycle: 8 + kind: 24) for two reasons:
//
// 1. Memory budget: circuits with 10⁹ T-gates produce ~10¹⁰ events.
//    At 32 B/event that's 320 GB; at 48 B it's 480 GB — a 50% increase
//    that can push a run from feasible to OOM.
//
// 2. Cache efficiency: 32 B = half a cache line. Two events fit in one
//    64-byte line during the analyzer's sequential scan. Larger events
//    reduce scan throughput.
//
// The largest variant is currently `FixupInserted { fixup: u64, original: u64 }`
// at 16 bytes of payload + discriminant + padding = 24 bytes for the enum.
//
// If you need to add a variant with a larger payload:
//   (a) Check if an existing field can be narrowed (e.g., u64 → u32).
//   (b) Consider boxing the payload: `LargeEvent(Box<LargePayload>)`.
//       This keeps the enum at 24 bytes (discriminant + pointer) at the
//       cost of one heap allocation per event of that variant.
//   (c) If the variant is rare, (b) is almost always the right choice.
//
const _: () = assert!(
    std::mem::size_of::<TraceEvent>() == 32,
    "TraceEvent size budget exceeded — see comment above for mitigation options"
);
const _: () = assert!(
    std::mem::size_of::<TraceEventKind>() == 24,
    "TraceEventKind size budget exceeded — see comment above for mitigation options"
);

/// Bit flag for synthetic (fixup) operation IDs in trace events.
///
/// Original circuit operations carry their IR `OpId` directly.
/// Fixup nodes injected by the engine carry `SYNTHETIC_ID_FLAG | counter`.
/// [`TraceEventKind::FixupInserted`] links the two: `original` is the IR OpId,
/// `fixup` is the synthetic ID.
pub const SYNTHETIC_ID_FLAG: u64 = 1 << 63;

/// A single timestamped event in the execution trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub cycle: u64,
    pub kind: TraceEventKind,
}

/// Classification of trace events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TraceEventKind {
    // Factory events
    FactoryStarted {
        factory_id: u16,
    },
    FactoryProduced {
        factory_id: u16,
    },
    FactoryFailed {
        factory_id: u16,
    },

    // Gate lifecycle
    GateReady {
        gate: u64,
    },
    GateScheduled {
        gate: u64,
    },
    GateStalled {
        gate: u64,
    },
    GateServed {
        gate: u64,
        wait: u32,
    },
    GateCompleted {
        gate: u64,
    },

    // Injection errors
    InjectionError {
        gate: u64,
    },
    FixupInserted {
        fixup: u64,
        original: u64,
    },
    FixupCompleted {
        fixup: u64,
    },

    // Buffer
    BufferEnqueue {
        occupancy: u32,
    },
    BufferDequeue {
        occupancy: u32,
    },
    BufferFull,

    // Routing (scalar model: latency events)
    RoutingStarted {
        gate: u64,
    },
    RoutingCompleted {
        gate: u64,
        latency: u32,
    },

    // Measurement hooks
    MeasurementOutcome {
        gate: u64,
        outcome: pirx_ir::circuit::MeasurementOutcome,
    },
    OpsActivated {
        gate: u64,
        activated_count: u32,
    },
}

/// Complete execution trace — the primary output of the engine.
///
/// A `Trace` is a complete record of everything that happened during
/// simulation. It can be serialized to disk, loaded later, and analyzed
/// by the `ProfileAnalyzer` without re-running the simulation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trace {
    pub schema_version: String,
    pub events: Vec<TraceEvent>,
    pub seed: u64,
    pub total_cycles: u64,
    /// True if simulation was stopped by `max_cycles` before all ops completed.
    #[serde(default)]
    pub truncated: bool,
    /// Logical error probability per consumed magic state, derived from QEC parameters.
    #[serde(default)]
    pub p_logical: f64,
    /// Total magic states consumed during simulation.
    #[serde(default)]
    pub magic_states_consumed: u64,
}

/// Append-only event accumulator. Pre-allocated with a best-effort hint.
/// Growth beyond the hint is amortized O(1) per event — acceptable for
/// the trace collector, which is not on the simulation critical path.
pub struct TraceCollector {
    events: Vec<TraceEvent>,
}

impl TraceCollector {
    /// Create a collector pre-allocated for `capacity_hint` events.
    pub fn new(capacity_hint: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity_hint),
        }
    }

    /// Append one event. Called on every engine step — must not allocate
    /// after the initial capacity is reached.
    #[inline]
    pub fn record(&mut self, cycle: u64, kind: TraceEventKind) {
        self.events.push(TraceEvent { cycle, kind });
    }

    /// Seal the event stream into an immutable `Trace` artifact.
    pub fn finish(
        self,
        seed: u64,
        total_cycles: u64,
        p_logical: f64,
        magic_states_consumed: u64,
    ) -> Trace {
        Trace {
            schema_version: "1.1".to_owned(),
            events: self.events,
            seed,
            total_cycles,
            truncated: false,
            p_logical,
            magic_states_consumed,
        }
    }

    /// Seal the event stream as a truncated trace (stopped by `max_cycles`).
    pub fn finish_truncated(
        self,
        seed: u64,
        total_cycles: u64,
        p_logical: f64,
        magic_states_consumed: u64,
    ) -> Trace {
        Trace {
            schema_version: "1.1".to_owned(),
            events: self.events,
            seed,
            total_cycles,
            truncated: true,
            p_logical,
            magic_states_consumed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceCollector, TraceEventKind};

    #[test]
    fn empty_trace() {
        let collector = TraceCollector::new(0);
        let trace = collector.finish(42, 0, 0.0, 0);
        assert!(trace.events.is_empty());
        assert_eq!(trace.seed, 42);
        assert_eq!(trace.total_cycles, 0);
        assert_eq!(trace.schema_version, "1.1");
    }

    #[test]
    fn records_in_order() {
        let mut collector = TraceCollector::new(4);
        collector.record(0, TraceEventKind::GateReady { gate: 1 });
        collector.record(1, TraceEventKind::GateScheduled { gate: 1 });
        collector.record(2, TraceEventKind::GateCompleted { gate: 1 });
        let trace = collector.finish(0, 2, 0.0, 0);
        let cycles: Vec<u64> = trace.events.iter().map(|e| e.cycle).collect();
        assert_eq!(cycles, [0, 1, 2]);
    }

    #[test]
    fn finish_truncated_sets_flag() {
        let mut collector = TraceCollector::new(2);
        collector.record(0, TraceEventKind::GateReady { gate: 1 });
        let trace = collector.finish_truncated(7, 10, 0.0, 0);
        assert!(trace.truncated);
        assert_eq!(trace.seed, 7);
        assert_eq!(trace.total_cycles, 10);
    }

    #[test]
    fn finish_normal_is_not_truncated() {
        let collector = TraceCollector::new(0);
        let trace = collector.finish(0, 5, 0.0, 0);
        assert!(!trace.truncated);
    }

    #[test]
    fn capacity_hint_does_not_affect_semantics() {
        let mut a = TraceCollector::new(0);
        let mut b = TraceCollector::new(1024);
        for col in [&mut a, &mut b] {
            col.record(5, TraceEventKind::FactoryStarted { factory_id: 0 });
            col.record(7, TraceEventKind::FactoryProduced { factory_id: 0 });
        }
        assert_eq!(a.finish(1, 10, 0.0, 0), b.finish(1, 10, 0.0, 0));
    }
}
