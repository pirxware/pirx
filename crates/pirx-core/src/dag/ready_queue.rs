//! Ready-gate scheduling — FIFO queue for the engine's ready set.

use std::collections::VecDeque;

use super::kind::OpKey;

/// FIFO ready queue — gates that become ready in the same cycle are served
/// in insertion order, matching the priority-list scheduling model and
/// ensuring determinism under a fixed seed.
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

    pub fn push(&mut self, key: OpKey) {
        self.inner.push_back(key);
    }

    pub fn pop(&mut self) -> Option<OpKey> {
        self.inner.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

impl Default for FifoReadyQueue {
    fn default() -> Self {
        Self::new()
    }
}
