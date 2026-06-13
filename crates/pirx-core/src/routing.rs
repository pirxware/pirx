//! Routing cost models — pluggable latency estimation for multi-qubit gates.

use pirx_hw::RoutingConfig;
use pirx_ir::circuit::{GridPosition, QubitId};

/// Flat position index: `position_index[qubit_id] = (row, col)`.
/// Built once in `Engine::new`, O(1) lookup per gate.
pub type PositionIndex = [(u32, u32)];

/// Routing cost model. Returns latency in QEC cycles.
pub(crate) trait RoutingModel: Send {
    fn latency(&self, qubits: &[QubitId], positions: &PositionIndex) -> u32;
}

/// Fixed overhead per multi-qubit gate. Ignores topology.
pub(crate) struct ScalarRouting {
    pub overhead_cycles: u32,
}

impl RoutingModel for ScalarRouting {
    fn latency(&self, qubits: &[QubitId], _positions: &PositionIndex) -> u32 {
        if qubits.len() < 2 {
            0
        } else {
            self.overhead_cycles
        }
    }
}

/// Manhattan distance on a logical qubit grid.
pub(crate) struct ManhattanRouting {
    pub cycles_per_hop: u32,
}

impl RoutingModel for ManhattanRouting {
    fn latency(&self, qubits: &[QubitId], positions: &PositionIndex) -> u32 {
        let (Some(&qa), Some(&qb)) = (qubits.first(), qubits.get(1)) else {
            return 0;
        };
        let pos_a = positions.get(qa as usize);
        let pos_b = positions.get(qb as usize);
        match (pos_a, pos_b) {
            (Some(&(ra, ca)), Some(&(rb, cb))) => ra
                .abs_diff(rb)
                .saturating_add(ca.abs_diff(cb))
                .saturating_mul(self.cycles_per_hop),
            _ => 0,
        }
    }
}

/// Build a flat position index from `GridPosition` slice.
/// Index is `qubit_id → (row, col)`, sized to `qubit_count`.
pub(crate) fn build_position_index(
    positions: &[GridPosition],
    qubit_count: u32,
) -> Vec<(u32, u32)> {
    let mut idx = vec![(0u32, 0u32); qubit_count as usize];
    for pos in positions {
        if let Some(slot) = idx.get_mut(pos.qubit as usize) {
            *slot = (pos.row, pos.col);
        }
    }
    idx
}

/// Enum dispatch for routing models — zero vtable overhead, inlineable.
pub(crate) enum RoutingKind {
    Scalar(ScalarRouting),
    Manhattan(ManhattanRouting),
}

impl RoutingModel for RoutingKind {
    fn latency(&self, qubits: &[QubitId], positions: &PositionIndex) -> u32 {
        match self {
            Self::Scalar(r) => r.latency(qubits, positions),
            Self::Manhattan(r) => r.latency(qubits, positions),
        }
    }
}

/// Build routing model from config. Scalar converts `overhead_fraction`
/// to a fixed cycle count (fraction × 10, rounded up).
pub(crate) fn from_config(config: &RoutingConfig) -> RoutingKind {
    match config {
        RoutingConfig::Scalar { overhead_fraction } => {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let overhead_cycles = (*overhead_fraction * 10.0).ceil() as u32;
            RoutingKind::Scalar(ScalarRouting { overhead_cycles })
        }
        RoutingConfig::Manhattan { cycles_per_hop, .. } => {
            RoutingKind::Manhattan(ManhattanRouting {
                cycles_per_hop: *cycles_per_hop,
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn two_qubit_index() -> Vec<(u32, u32)> {
        vec![(0, 0), (0, 1)]
    }

    // ── ScalarRouting ────────────────────────────────────────────────────────

    #[test]
    fn scalar_single_qubit_zero_cost() {
        let r = ScalarRouting { overhead_cycles: 5 };
        assert_eq!(r.latency(&[0], &[]), 0);
    }

    #[test]
    fn scalar_two_qubit_fixed_cost() {
        let r = ScalarRouting { overhead_cycles: 5 };
        assert_eq!(r.latency(&[0, 1], &two_qubit_index()), 5);
    }

    // ── ManhattanRouting ─────────────────────────────────────────────────────

    #[test]
    fn manhattan_single_qubit_zero_cost() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        assert_eq!(r.latency(&[0], &two_qubit_index()), 0);
    }

    #[test]
    fn manhattan_adjacent_qubits() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        // (0,0) → (0,1): distance = 1
        assert_eq!(r.latency(&[0, 1], &two_qubit_index()), 1);
    }

    #[test]
    fn manhattan_distant_qubits() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        let positions = vec![(0, 0), (5, 7)];
        // |5-0| + |7-0| = 12
        assert_eq!(r.latency(&[0, 1], &positions), 12);
    }

    #[test]
    fn manhattan_cycles_per_hop_multiplier() {
        let r = ManhattanRouting { cycles_per_hop: 3 };
        let positions = vec![(0, 0), (2, 1)];
        // distance = 3, × 3 cycles/hop = 9
        assert_eq!(r.latency(&[0, 1], &positions), 9);
    }

    #[test]
    fn manhattan_missing_position_returns_zero() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        // qubit 5 out of bounds
        assert_eq!(r.latency(&[0, 5], &two_qubit_index()), 0);
    }

    // ── build_position_index ─────────────────────────────────────────────────

    #[test]
    fn build_index_maps_positions() {
        let positions = vec![
            GridPosition {
                qubit: 0,
                row: 3,
                col: 7,
            },
            GridPosition {
                qubit: 2,
                row: 1,
                col: 4,
            },
        ];
        let idx = build_position_index(&positions, 3);
        assert_eq!(idx, vec![(3, 7), (0, 0), (1, 4)]);
    }

    // ── from_config ──────────────────────────────────────────────────────────

    #[test]
    fn from_config_scalar() {
        let cfg = RoutingConfig::Scalar {
            overhead_fraction: 0.5,
        };
        let model = from_config(&cfg);
        // 0.5 × 10 = 5.0, ceil = 5
        assert_eq!(model.latency(&[0, 1], &two_qubit_index()), 5);
    }

    #[test]
    fn from_config_manhattan() {
        let cfg = RoutingConfig::Manhattan {
            grid_width: 10,
            grid_height: 10,
            cycles_per_hop: 2,
        };
        let model = from_config(&cfg);
        let positions = vec![(0, 0), (1, 1)];
        // distance = 2, × 2 = 4
        assert_eq!(model.latency(&[0, 1], &positions), 4);
    }
}
