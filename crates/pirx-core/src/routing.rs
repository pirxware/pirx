//! Routing cost models — pluggable latency estimation for multi-qubit gates.

use pirx_hw::RoutingConfig;
use pirx_ir::circuit::{GridPosition, QubitId};

/// Routing cost model. Returns latency in QEC cycles.
pub trait RoutingModel: Send {
    fn latency(&self, qubits: &[QubitId], positions: &[GridPosition]) -> u32;
}

/// Fixed overhead per multi-qubit gate. Ignores topology.
pub struct ScalarRouting {
    pub overhead_cycles: u32,
}

impl RoutingModel for ScalarRouting {
    fn latency(&self, qubits: &[QubitId], _positions: &[GridPosition]) -> u32 {
        if qubits.len() < 2 {
            0
        } else {
            self.overhead_cycles
        }
    }
}

/// Manhattan distance on a logical qubit grid.
pub struct ManhattanRouting {
    pub cycles_per_hop: u32,
}

impl RoutingModel for ManhattanRouting {
    fn latency(&self, qubits: &[QubitId], positions: &[GridPosition]) -> u32 {
        let (Some(&qa), Some(&qb)) = (qubits.first(), qubits.get(1)) else {
            return 0;
        };
        let pos_a = positions.iter().find(|p| p.qubit == qa);
        let pos_b = positions.iter().find(|p| p.qubit == qb);
        match (pos_a, pos_b) {
            (Some(a), Some(b)) => {
                (a.row.abs_diff(b.row) + a.col.abs_diff(b.col)) * self.cycles_per_hop
            }
            _ => 0,
        }
    }
}

/// Build routing model from config. Scalar converts `overhead_fraction`
/// to a fixed cycle count (fraction × 10, rounded up).
pub fn from_config(config: &RoutingConfig) -> Box<dyn RoutingModel> {
    match config {
        RoutingConfig::Scalar { overhead_fraction } => {
            // overhead_fraction is validated to [0.0, 1.0] by
            // HardwareModel::validate(), so the product is in [0.0, 10.0].
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let overhead_cycles = (*overhead_fraction * 10.0).ceil() as u32;
            Box::new(ScalarRouting { overhead_cycles })
        }
        RoutingConfig::Manhattan { cycles_per_hop, .. } => Box::new(ManhattanRouting {
            cycles_per_hop: *cycles_per_hop,
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use pirx_ir::circuit::GridPosition;

    use super::*;

    fn two_qubit_positions() -> Vec<GridPosition> {
        vec![
            GridPosition {
                qubit: 0,
                row: 0,
                col: 0,
            },
            GridPosition {
                qubit: 1,
                row: 0,
                col: 1,
            },
        ]
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
        assert_eq!(r.latency(&[0, 1], &two_qubit_positions()), 5);
    }

    // ── ManhattanRouting ─────────────────────────────────────────────────────

    #[test]
    fn manhattan_single_qubit_zero_cost() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        assert_eq!(r.latency(&[0], &two_qubit_positions()), 0);
    }

    #[test]
    fn manhattan_adjacent_qubits() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        // (0,0) → (0,1): distance = 1
        assert_eq!(r.latency(&[0, 1], &two_qubit_positions()), 1);
    }

    #[test]
    fn manhattan_distant_qubits() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        let positions = vec![
            GridPosition {
                qubit: 0,
                row: 0,
                col: 0,
            },
            GridPosition {
                qubit: 1,
                row: 5,
                col: 7,
            },
        ];
        // |5-0| + |7-0| = 12
        assert_eq!(r.latency(&[0, 1], &positions), 12);
    }

    #[test]
    fn manhattan_cycles_per_hop_multiplier() {
        let r = ManhattanRouting { cycles_per_hop: 3 };
        let positions = vec![
            GridPosition {
                qubit: 0,
                row: 0,
                col: 0,
            },
            GridPosition {
                qubit: 1,
                row: 2,
                col: 1,
            },
        ];
        // distance = 3, × 3 cycles/hop = 9
        assert_eq!(r.latency(&[0, 1], &positions), 9);
    }

    #[test]
    fn manhattan_missing_position_returns_zero() {
        let r = ManhattanRouting { cycles_per_hop: 1 };
        // qubit 5 has no position entry
        assert_eq!(r.latency(&[0, 5], &two_qubit_positions()), 0);
    }

    // ── from_config ──────────────────────────────────────────────────────────

    #[test]
    fn from_config_scalar() {
        let cfg = RoutingConfig::Scalar {
            overhead_fraction: 0.5,
        };
        let model = from_config(&cfg);
        // 0.5 × 10 = 5.0, ceil = 5
        assert_eq!(model.latency(&[0, 1], &two_qubit_positions()), 5);
    }

    #[test]
    fn from_config_manhattan() {
        let cfg = RoutingConfig::Manhattan {
            grid_width: 10,
            grid_height: 10,
            cycles_per_hop: 2,
        };
        let model = from_config(&cfg);
        let positions = vec![
            GridPosition {
                qubit: 0,
                row: 0,
                col: 0,
            },
            GridPosition {
                qubit: 1,
                row: 1,
                col: 1,
            },
        ];
        // distance = 2, × 2 = 4
        assert_eq!(model.latency(&[0, 1], &positions), 4);
    }
}
