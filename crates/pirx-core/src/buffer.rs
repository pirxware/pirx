//! Magic state buffer — the bounded counter between factories and T-gates.

/// Fixed-capacity counter tracking the number of magic states ready for injection.
///
/// Full => factory output is wasted. Empty => T-gate stalls. No allocation; state is
/// a single bounded counter.
pub(crate) struct MagicStateBuffer {
    capacity: u32,
    occupancy: u32,
}

impl MagicStateBuffer {
    /// Create a buffer with the given capacity, pre-loaded with up to `preload` states.
    ///
    /// `preload` is clamped to `capacity` so the buffer is always valid on construction.
    pub fn new(capacity: u32, preload: u32) -> Self {
        Self {
            capacity,
            occupancy: preload.min(capacity),
        }
    }

    /// Try to enqueue a produced magic state. Returns `false` if the buffer is full
    /// (factory output wasted).
    #[inline]
    pub fn try_enqueue(&mut self) -> bool {
        if self.occupancy < self.capacity {
            self.occupancy += 1;
            true
        } else {
            false
        }
    }

    /// Try to dequeue a magic state for a T-gate. Returns `false` if the buffer is empty
    /// (gate must stall).
    #[inline]
    pub fn try_dequeue(&mut self) -> bool {
        if self.occupancy > 0 {
            self.occupancy -= 1;
            true
        } else {
            false
        }
    }

    /// Current number of magic states in the buffer.
    #[inline]
    pub fn occupancy(&self) -> u32 {
        self.occupancy
    }

    /// `true` when no more states can be enqueued.
    #[cfg(test)]
    #[inline]
    pub fn is_full(&self) -> bool {
        self.occupancy >= self.capacity
    }

    /// `true` when no states are available for dequeue.
    #[cfg(test)]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.occupancy == 0
    }
}

#[cfg(test)]
mod tests {
    use super::MagicStateBuffer;

    #[test]
    fn new_clamps_preload() {
        let buf = MagicStateBuffer::new(4, 10);
        assert_eq!(buf.occupancy(), 4);
        assert!(buf.is_full());
    }

    #[test]
    fn enqueue_dequeue_basic() {
        let mut buf = MagicStateBuffer::new(4, 0);
        assert!(buf.try_enqueue());
        assert_eq!(buf.occupancy(), 1);
        assert!(buf.try_dequeue());
        assert_eq!(buf.occupancy(), 0);
    }

    #[test]
    fn enqueue_full_returns_false() {
        let mut buf = MagicStateBuffer::new(2, 2);
        assert!(!buf.try_enqueue());
        assert_eq!(buf.occupancy(), 2);
    }

    #[test]
    fn dequeue_empty_returns_false() {
        let mut buf = MagicStateBuffer::new(4, 0);
        assert!(!buf.try_dequeue());
        assert_eq!(buf.occupancy(), 0);
    }

    #[test]
    fn cold_start() {
        let buf = MagicStateBuffer::new(8, 0);
        assert!(buf.is_empty());
        assert!(!buf.is_full());
        assert_eq!(buf.occupancy(), 0);
    }

    #[test]
    fn warm_start() {
        let buf = MagicStateBuffer::new(8, 3);
        assert!(!buf.is_empty());
        assert!(!buf.is_full());
        assert_eq!(buf.occupancy(), 3);
    }
}
