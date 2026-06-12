//! Trace event types — collected during simulation, analyzed after.
//!
//! 24 bytes per event (cycle: 8 + kind: 16). Append-only during
//! simulation, immutable after. Analyzed by the profile analyzer.

use serde::{Deserialize, Serialize};

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
    pub fn finish(self, seed: u64, total_cycles: u64) -> Trace {
        Trace {
            schema_version: "1.0".to_owned(),
            events: self.events,
            seed,
            total_cycles,
            truncated: false,
        }
    }

    /// Seal the event stream as a truncated trace (stopped by `max_cycles`).
    pub fn finish_truncated(self, seed: u64, total_cycles: u64) -> Trace {
        Trace {
            schema_version: "1.0".to_owned(),
            events: self.events,
            seed,
            total_cycles,
            truncated: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceCollector, TraceEventKind};

    #[test]
    fn empty_trace() {
        let collector = TraceCollector::new(0);
        let trace = collector.finish(42, 0);
        assert!(trace.events.is_empty());
        assert_eq!(trace.seed, 42);
        assert_eq!(trace.total_cycles, 0);
        assert_eq!(trace.schema_version, "1.0");
    }

    #[test]
    fn records_in_order() {
        let mut collector = TraceCollector::new(4);
        collector.record(0, TraceEventKind::GateReady { gate: 1 });
        collector.record(1, TraceEventKind::GateScheduled { gate: 1 });
        collector.record(2, TraceEventKind::GateCompleted { gate: 1 });
        let trace = collector.finish(0, 2);
        let cycles: Vec<u64> = trace.events.iter().map(|e| e.cycle).collect();
        assert_eq!(cycles, [0, 1, 2]);
    }

    #[test]
    fn finish_truncated_sets_flag() {
        let mut collector = TraceCollector::new(2);
        collector.record(0, TraceEventKind::GateReady { gate: 1 });
        let trace = collector.finish_truncated(7, 10);
        assert!(trace.truncated);
        assert_eq!(trace.seed, 7);
        assert_eq!(trace.total_cycles, 10);
    }

    #[test]
    fn finish_normal_is_not_truncated() {
        let collector = TraceCollector::new(0);
        let trace = collector.finish(0, 5);
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
        assert_eq!(a.finish(1, 10), b.finish(1, 10));
    }
}
