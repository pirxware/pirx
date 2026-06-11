//! Trace event types — collected during simulation, analyzed after.
//!
//! 24 bytes per event (cycle: 8 + kind: 16). Append-only during
//! simulation, immutable after. Analyzed by the profile analyzer.

use serde::{Deserialize, Serialize};

/// A single timestamped event in the execution trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub cycle: u64,
    pub kind: TraceEventKind,
}

/// Classification of trace events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceEventKind {
    // Factory events
    FactoryStarted { factory_id: u16 },
    FactoryProduced { factory_id: u16 },
    FactoryFailed { factory_id: u16 },

    // Gate lifecycle
    GateReady { gate: u64 },
    GateScheduled { gate: u64 },
    GateStalled { gate: u64 },
    GateServed { gate: u64, wait: u32 },
    GateCompleted { gate: u64 },

    // Injection errors
    InjectionError { gate: u64 },
    FixupInserted { fixup: u64, original: u64 },
    FixupCompleted { fixup: u64 },

    // Buffer
    BufferEnqueue { occupancy: u32 },
    BufferDequeue { occupancy: u32 },
    BufferFull,

    // Routing (scalar model: latency events)
    RoutingStarted { gate: u64 },
    RoutingCompleted { gate: u64, latency: u32 },
}

/// Complete execution trace — the primary output of the engine.
///
/// A `Trace` is a complete record of everything that happened during
/// simulation. It can be serialized to disk, loaded later, and analyzed
/// by the `ProfileAnalyzer` without re-running the simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub events: Vec<TraceEvent>,
    pub seed: u64,
    pub total_cycles: u64,
}
