//! Engine event types and priority queue.
//!
//! Three event variants represent the only deferred state transitions:
//! factory completion, factory abort, and gate execution. Everything else
//! is handled synchronously within the cycle loop.

use std::{cmp::Reverse, collections::BinaryHeap};

use pirx_ir::circuit::{MeasurementHookId, MeasurementOutcome};

use crate::dag::OpKey;

/// An engine event — a deferred state transition completing in a future cycle.
///
/// Only things with non-zero latency are queued. Synchronous state changes
/// (injection error insertion, buffer updates) happen directly in the cycle
/// loop without going through the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineEvent {
    /// Factory finishes producing a magic state.
    FactoryProduced { factory_id: u16 },
    /// Factory production attempt fails (abort/restart).
    FactoryFailed { factory_id: u16 },
    /// A gate's cycle cost has elapsed — it completes.
    GateCompleted { gate: OpKey },
    /// Deferred measurement hook activation after classical feedback delay.
    HookActivation {
        gate: OpKey,
        hook_id: MeasurementHookId,
        outcome: MeasurementOutcome,
    },
}

/// A timestamped, sequenced engine event for priority-queue ordering.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimedEvent {
    pub cycle: u64,
    /// Monotonic insertion counter; never reset. Breaks ties within a cycle
    /// deterministically — events generated earlier in the same cycle process
    /// first, matching insertion order and satisfying P1 reproducibility.
    pub seq: u64,
    pub event: EngineEvent,
}

impl PartialEq for TimedEvent {
    fn eq(&self, other: &Self) -> bool {
        (self.cycle, self.seq) == (other.cycle, other.seq)
    }
}

impl Eq for TimedEvent {}

impl PartialOrd for TimedEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimedEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.cycle, self.seq).cmp(&(other.cycle, other.seq))
    }
}

/// Min-heap priority queue ordered by `(cycle, seq)`.
///
/// Deterministic: same event generation order → same seq values → same
/// processing order → same trace.
pub(crate) struct EventQueue {
    heap: BinaryHeap<Reverse<TimedEvent>>,
    next_seq: u64,
}

impl EventQueue {
    /// Create an empty event queue.
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            next_seq: 0,
        }
    }

    /// Create an event queue pre-allocated for `capacity` events.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(capacity),
            next_seq: 0,
        }
    }

    /// Schedule an event to be delivered at `cycle`.
    pub fn schedule(&mut self, cycle: u64, event: EngineEvent) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.heap.push(Reverse(TimedEvent { cycle, seq, event }));
    }

    /// Pop the earliest event (lowest cycle, then lowest seq).
    pub fn pop(&mut self) -> Option<TimedEvent> {
        self.heap.pop().map(|Reverse(e)| e)
    }

    /// Return the cycle of the next event without consuming it.
    pub fn peek_cycle(&self) -> Option<u64> {
        self.heap.peek().map(|Reverse(e)| e.cycle)
    }

    /// True when no events are pending.
    #[cfg(test)]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use slotmap::SlotMap;

    use super::{EngineEvent, EventQueue, OpKey};

    fn factory_produced(id: u16) -> EngineEvent {
        EngineEvent::FactoryProduced { factory_id: id }
    }

    #[test]
    fn empty_queue() {
        let mut q = EventQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.peek_cycle(), None);
        assert!(q.pop().is_none());
    }

    #[test]
    fn pop_ordered_by_cycle() {
        let mut q = EventQueue::new();
        q.schedule(3, factory_produced(0));
        q.schedule(1, factory_produced(1));
        q.schedule(2, factory_produced(2));

        let cycles: Vec<u64> = (0..3).filter_map(|_| q.pop().map(|e| e.cycle)).collect();
        assert_eq!(cycles, [1, 2, 3]);
        assert!(q.is_empty());
    }

    #[test]
    fn same_cycle_ordered_by_seq() {
        // Create distinct OpKey values via a temporary SlotMap (can't construct them directly).
        let mut map: SlotMap<OpKey, ()> = SlotMap::with_key();
        let k0 = map.insert(());
        let k1 = map.insert(());
        let k2 = map.insert(());

        let mut q = EventQueue::new();
        q.schedule(5, EngineEvent::GateCompleted { gate: k0 });
        q.schedule(5, EngineEvent::GateCompleted { gate: k1 });
        q.schedule(5, EngineEvent::GateCompleted { gate: k2 });

        // seq 0, 1, 2 — must come out in insertion order
        let seqs: Vec<u64> = (0..3).filter_map(|_| q.pop().map(|e| e.seq)).collect();
        assert_eq!(seqs, [0, 1, 2]);
    }

    #[test]
    fn seq_distinct_past_u32_max() {
        let mut q = EventQueue::new();
        q.next_seq = u64::from(u32::MAX) - 5;

        for _ in 0..10 {
            q.schedule(1, factory_produced(0));
        }

        let seqs: Vec<u64> = std::iter::from_fn(|| q.pop().map(|e| e.seq)).collect();
        assert_eq!(seqs.len(), 10);
        for window in seqs.windows(2) {
            assert!(
                window[1] > window[0],
                "seq must be strictly increasing across u32::MAX boundary: {} vs {}",
                window[0],
                window[1]
            );
        }
        assert!(
            seqs.last().copied().unwrap() > u64::from(u32::MAX),
            "seq must cross u32::MAX without saturating"
        );
    }

    #[test]
    fn peek_cycle_does_not_consume() {
        let mut q = EventQueue::new();
        q.schedule(10, factory_produced(0));
        assert_eq!(q.peek_cycle(), Some(10));
        assert_eq!(q.peek_cycle(), Some(10));
        assert!(q.pop().is_some());
        assert_eq!(q.peek_cycle(), None);
    }
}
