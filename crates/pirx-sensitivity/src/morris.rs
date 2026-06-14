//! Morris elementary effects method.

#![allow(dead_code)]

use rand::Rng as _;
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};

use crate::{config::MorrisConfig, error::SensitivityError};

/// Results of a Morris elementary effects analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorrisResult {
    pub parameters: Vec<MorrisParameterResult>,
    pub evaluations: u64,
    pub config: MorrisConfig,
}

impl MorrisResult {
    /// Parameters ranked by descending mu_star (most influential first).
    pub fn rankings(&self) -> Vec<&MorrisParameterResult> {
        let mut ranked: Vec<_> = self.parameters.iter().collect();
        ranked.sort_by(|a, b| {
            b.mu_star
                .partial_cmp(&a.mu_star)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked
    }
}

/// Morris sensitivity indices for a single parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MorrisParameterResult {
    pub name: String,
    /// Mean of elementary effects.
    pub mu: f64,
    /// Mean of |elementary effects| — primary importance measure.
    pub mu_star: f64,
    /// Standard deviation of elementary effects — high sigma indicates nonlinearity/interactions.
    pub sigma: f64,
    /// Raw elementary effects collected across all trajectories.
    pub elementary_effects: Vec<f64>,
}

impl MorrisConfig {
    /// Validate Morris configuration parameters.
    pub fn validate(self) -> Result<(), SensitivityError> {
        if self.trajectories < 2 {
            return Err(SensitivityError::InsufficientTrajectories(
                self.trajectories,
            ));
        }
        if self.levels < 4 || !self.levels.is_multiple_of(2) {
            return Err(SensitivityError::InvalidMorrisLevels(self.levels));
        }
        Ok(())
    }
}

/// Morris step size: delta = p / (2 * (p - 1)).
/// For levels=4: delta = 4/6 = 2/3. levels must be even for grid alignment.
pub(crate) fn delta_value(levels: u32) -> f64 {
    f64::from(levels) / (2.0 * (f64::from(levels) - 1.0))
}

pub(crate) struct MorrisTrajectory {
    pub(crate) points: Vec<Vec<f64>>,
    pub(crate) permutation: Vec<usize>,
    pub(crate) signs: Vec<f64>,
}

/// Generate Morris trajectories in the unit hypercube.
///
/// Base points are sampled from the grid {0, step, ..., 1-Δ}, ensuring
/// a +Δ perturbation stays within [0,1]. The sign is chosen adaptively:
/// a random preference is used when both directions are valid, otherwise
/// the sign is forced to the valid direction. This guarantees no clamping
/// and no zero-perturbation artifacts.
#[allow(clippy::indexing_slicing)]
pub(crate) fn generate_trajectories(
    dim: usize,
    config: MorrisConfig,
    rng: &mut ChaCha12Rng,
) -> Vec<MorrisTrajectory> {
    let delta = delta_value(config.levels);
    let grid_step = 1.0 / (f64::from(config.levels) - 1.0);

    // Base grid: {0, step, 2*step, ..., 1}
    let base_grid: Vec<f64> = {
        let mut vals = Vec::new();
        let mut v = 0.0;
        while v <= 1.0 + 1e-12 {
            vals.push(v);
            v += grid_step;
        }
        vals
    };

    (0..config.trajectories)
        .map(|_| {
            let mut point: Vec<f64> = (0..dim)
                .map(|_| base_grid[rng.random_range(0..base_grid.len())])
                .collect();

            let mut permutation: Vec<usize> = (0..dim).collect();
            for i in (1..dim).rev() {
                let j = rng.random_range(0..=i);
                permutation.swap(i, j);
            }

            let mut points = Vec::with_capacity(dim + 1);
            let mut signs = Vec::with_capacity(dim);
            points.push(point.clone());

            for &param_idx in &permutation {
                let preferred: f64 = if rng.random::<bool>() { 1.0 } else { -1.0 };
                let can_up = point[param_idx] + delta <= 1.0 + 1e-12;
                let can_down = point[param_idx] - delta >= -1e-12;
                let sign = match (can_up, can_down) {
                    (true, true) => preferred,
                    (true, false) => 1.0,
                    (false, true) => -1.0,
                    // Both valid at grid boundary — prefer up (unreachable for valid grids)
                    (false, false) => preferred,
                };
                point[param_idx] += sign * delta;
                point[param_idx] = point[param_idx].clamp(0.0, 1.0);
                debug_assert!((-1e-12..=1.0 + 1e-12).contains(&point[param_idx]));
                points.push(point.clone());
                signs.push(sign);
            }

            MorrisTrajectory {
                points,
                permutation,
                signs,
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]
mod tests {
    use rand::SeedableRng;

    use super::*;

    #[test]
    fn delta_value_levels_4() {
        let d = delta_value(4);
        assert!((d - 2.0 / 3.0).abs() < 1e-12, "expected 2/3, got {d}");
    }

    #[test]
    fn delta_value_levels_6() {
        let d = delta_value(6);
        assert!((d - 0.6).abs() < 1e-12, "expected 0.6, got {d}");
    }

    #[test]
    fn rejects_odd_levels() {
        let config = MorrisConfig {
            trajectories: 10,
            levels: 5,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, SensitivityError::InvalidMorrisLevels(5)),
            "expected InvalidMorrisLevels(5), got {err:?}"
        );
    }

    #[test]
    fn rejects_levels_below_4() {
        let config = MorrisConfig {
            trajectories: 10,
            levels: 2,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, SensitivityError::InvalidMorrisLevels(2)),
            "expected InvalidMorrisLevels(2), got {err:?}"
        );
    }

    #[test]
    fn accepts_even_levels() {
        for levels in [4, 6, 8] {
            let config = MorrisConfig {
                trajectories: 10,
                levels,
            };
            assert!(config.validate().is_ok(), "levels={levels} should be valid");
        }
    }

    #[test]
    fn rejects_insufficient_trajectories() {
        let config = MorrisConfig {
            trajectories: 1,
            levels: 4,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, SensitivityError::InsufficientTrajectories(1)),
            "expected InsufficientTrajectories(1), got {err:?}"
        );
    }

    #[test]
    fn trajectory_length() {
        let dim = 5;
        let config = MorrisConfig {
            trajectories: 3,
            levels: 4,
        };
        let mut rng = ChaCha12Rng::seed_from_u64(42);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        assert_eq!(trajectories.len(), 3);
        for traj in &trajectories {
            assert_eq!(
                traj.points.len(),
                dim + 1,
                "trajectory should have P+1 points"
            );
            for point in &traj.points {
                assert_eq!(point.len(), dim, "each point should be P-dimensional");
            }
        }
    }

    #[test]
    fn trajectory_permutation_complete() {
        let dim = 7;
        let config = MorrisConfig {
            trajectories: 5,
            levels: 4,
        };
        let mut rng = ChaCha12Rng::seed_from_u64(99);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        for traj in &trajectories {
            let mut sorted = traj.permutation.clone();
            sorted.sort_unstable();
            let expected: Vec<usize> = (0..dim).collect();
            assert_eq!(
                sorted, expected,
                "permutation must contain each param exactly once"
            );
        }
    }

    #[test]
    fn trajectory_points_on_grid() {
        let dim = 4;
        let config = MorrisConfig {
            trajectories: 10,
            levels: 4,
        };
        let grid_step = 1.0 / (f64::from(config.levels) - 1.0);
        let mut rng = ChaCha12Rng::seed_from_u64(7);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        for traj in &trajectories {
            for point in &traj.points {
                for &coord in point {
                    let remainder = (coord / grid_step).round() * grid_step - coord;
                    assert!(
                        remainder.abs() < 1e-10,
                        "coordinate {coord} is not a multiple of grid_step={grid_step}"
                    );
                }
            }
        }
    }

    #[test]
    fn trajectory_perturbation_magnitude() {
        let dim = 5;
        let config = MorrisConfig {
            trajectories: 10,
            levels: 4,
        };
        let delta = delta_value(config.levels);
        let mut rng = ChaCha12Rng::seed_from_u64(123);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        for traj in &trajectories {
            for i in 0..dim {
                let diff: f64 = traj.points[i + 1]
                    .iter()
                    .zip(traj.points[i].iter())
                    .map(|(a, b)| (a - b).abs())
                    .sum();
                assert!(
                    (diff - delta).abs() < 1e-10,
                    "step {i}: expected perturbation {delta}, got {diff}"
                );
            }
        }
    }

    #[test]
    fn trajectory_determinism() {
        let dim = 4;
        let config = MorrisConfig {
            trajectories: 5,
            levels: 6,
        };

        let mut rng1 = ChaCha12Rng::seed_from_u64(42);
        let t1 = generate_trajectories(dim, config, &mut rng1);

        let mut rng2 = ChaCha12Rng::seed_from_u64(42);
        let t2 = generate_trajectories(dim, config, &mut rng2);

        for (a, b) in t1.iter().zip(t2.iter()) {
            assert_eq!(a.points, b.points);
            assert_eq!(a.permutation, b.permutation);
            assert_eq!(a.signs, b.signs);
        }
    }

    #[test]
    fn trajectory_bounds() {
        let dim = 6;
        let config = MorrisConfig {
            trajectories: 20,
            levels: 4,
        };
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        for traj in &trajectories {
            for point in &traj.points {
                for &coord in point {
                    assert!(
                        (-1e-12..=1.0 + 1e-12).contains(&coord),
                        "coordinate {coord} out of [0, 1]"
                    );
                }
            }
        }
    }

    #[test]
    fn trajectory_no_zero_perturbation() {
        let dim = 4;
        let config = MorrisConfig {
            trajectories: 50,
            levels: 4,
        };
        let mut rng = ChaCha12Rng::seed_from_u64(777);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        for traj in &trajectories {
            for i in 0..dim {
                let identical = traj.points[i]
                    .iter()
                    .zip(traj.points[i + 1].iter())
                    .all(|(a, b)| (a - b).abs() < 1e-15);
                assert!(
                    !identical,
                    "consecutive points {i} and {} are identical — zero perturbation",
                    i + 1
                );
            }
        }
    }
}
