//! Ready-gate scheduling policy.

use std::collections::VecDeque;

use super::kind::OpKey;

/// Interface for the ready-gate queue.
///
/// The default implementation is [`FifoReadyQueue`]. The trait exists so that
/// future priority-scheduling policies (critical-path-first, T-gate-first) can
/// be swapped in without changing the engine loop.
pub trait ReadyQueue {
    fn push(&mut self, key: OpKey);
    fn pop(&mut self) -> Option<OpKey>;
    fn is_empty(&self) -> bool;
    fn len(&self) -> usize;
}

/// FIFO ready queue — the default scheduling policy.
///
/// Gates that become ready in the same cycle are served in insertion order,
/// matching the priority-list scheduling model and ensuring determinism under
/// a fixed seed.
#[derive(Debug)]
pub struct FifoReadyQueue {
    inner: VecDeque<OpKey>,
}

impl FifoReadyQueue {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(capacity),
        }
    }
}

impl Default for FifoReadyQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadyQueue for FifoReadyQueue {
    fn push(&mut self, key: OpKey) {
        self.inner.push_back(key);
    }

    fn pop(&mut self) -> Option<OpKey> {
        self.inner.pop_front()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}
