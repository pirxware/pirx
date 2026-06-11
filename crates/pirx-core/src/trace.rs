//! Trace event types — collected during simulation, analyzed after.

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
    FactoryStarted { factory_id: u16 },
    FactoryProduced { factory_id: u16 },
    FactoryFailed { factory_id: u16 },
    GateReady { gate: u64 },
    GateScheduled { gate: u64 },
    GateStalled { gate: u64 },
    GateServed { gate: u64, wait: u32 },
    GateCompleted { gate: u64 },
    InjectionError { gate: u64 },
    FixupInserted { fixup: u64, original: u64 },
    FixupCompleted { fixup: u64 },
    BufferEnqueue { occupancy: u16 },
    BufferDequeue { occupancy: u16 },
    BufferFull,
}

/// Complete execution trace — the primary output of the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub events: Vec<TraceEvent>,
}
