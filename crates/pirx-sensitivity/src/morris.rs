//! Morris elementary effects method.

#![allow(dead_code)]

use pirx_hw::model::HardwareModel;
use pirx_ir::ValidatedCircuit;
use rand::{Rng as _, SeedableRng};
use rand_chacha::ChaCha12Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    config::MorrisConfig,
    error::SensitivityError,
    parameter::ParameterSpace,
    sample::{EvalConfig, evaluate_point},
};

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

/// Extract elementary effects from pre-computed function values.
#[allow(clippy::indexing_slicing)]
pub(crate) fn extract_elementary_effects(
    all_values: &[f64],
    trajectories: &[MorrisTrajectory],
    dim: usize,
    levels: u32,
) -> Vec<Vec<f64>> {
    let delta = delta_value(levels);
    let points_per_traj = dim + 1;
    let mut ee_per_param: Vec<Vec<f64>> = vec![Vec::with_capacity(trajectories.len()); dim];

    for (t, traj) in trajectories.iter().enumerate() {
        let base_offset = t * points_per_traj;
        for (j, &param_idx) in traj.permutation.iter().enumerate() {
            let f_before = all_values[base_offset + j];
            let f_after = all_values[base_offset + j + 1];
            let delta_signed = traj.signs[j] * delta;
            let ee = (f_after - f_before) / delta_signed;
            ee_per_param[param_idx].push(ee);
        }
    }

    ee_per_param
}

/// Evaluate all trajectory points in parallel and extract elementary effects.
#[allow(clippy::indexing_slicing)]
pub(crate) fn evaluate_trajectories(
    trajectories: &[MorrisTrajectory],
    circuit: &ValidatedCircuit,
    base_hw: &HardwareModel,
    space: &ParameterSpace,
    eval_config: &EvalConfig,
    config: MorrisConfig,
) -> Result<Vec<Vec<f64>>, SensitivityError> {
    let points_per_traj = space.dim() + 1;
    let total_points = trajectories.len() * points_per_traj;

    let all_values: Vec<f64> = (0..total_points)
        .into_par_iter()
        .map(|flat_idx| {
            let t = flat_idx / points_per_traj;
            let s = flat_idx % points_per_traj;
            evaluate_point(
                circuit,
                base_hw,
                space,
                &trajectories[t].points[s],
                flat_idx,
                eval_config,
            )
        })
        .collect::<Result<_, _>>()?;

    Ok(extract_elementary_effects(
        &all_values,
        trajectories,
        space.dim(),
        config.levels,
    ))
}

/// Aggregate elementary effects into Morris sensitivity indices.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn aggregate_morris(
    ee_per_param: Vec<Vec<f64>>,
    space: &ParameterSpace,
    config: MorrisConfig,
) -> MorrisResult {
    let parameters = ee_per_param
        .into_iter()
        .zip(space.params().iter())
        .map(|(ees, param)| {
            let n = ees.len() as f64;
            let mu = ees.iter().sum::<f64>() / n;
            let mu_star = ees.iter().map(|e| e.abs()).sum::<f64>() / n;
            let sigma = if n > 1.0 {
                (ees.iter().map(|e| (e - mu).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
            } else {
                0.0
            };

            MorrisParameterResult {
                name: param.name.clone(),
                mu,
                mu_star,
                sigma,
                elementary_effects: ees,
            }
        })
        .collect();

    MorrisResult {
        parameters,
        evaluations: u64::from(config.trajectories) * (space.dim() as u64 + 1),
        config,
    }
}

/// Run a complete Morris elementary effects screening analysis.
pub fn morris_screening(
    circuit: &ValidatedCircuit,
    base_hw: &HardwareModel,
    space: &ParameterSpace,
    eval_config: &EvalConfig,
    config: MorrisConfig,
) -> Result<MorrisResult, SensitivityError> {
    config.validate()?;
    if space.dim() == 0 {
        return Err(SensitivityError::EmptyParameterSpace);
    }

    let mut rng = ChaCha12Rng::seed_from_u64(eval_config.base_seed);
    let trajectories = generate_trajectories(space.dim(), config, &mut rng);
    let ee_per_param =
        evaluate_trajectories(&trajectories, circuit, base_hw, space, eval_config, config)?;
    Ok(aggregate_morris(ee_per_param, space, config))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
mod tests {
    use rand::SeedableRng;

    use super::*;
    use crate::parameter::{ParameterDef, ParameterKind, ParameterSpace};

    fn default_config() -> MorrisConfig {
        MorrisConfig {
            trajectories: 20,
            levels: 4,
        }
    }

    fn synthetic_screening<F>(dim: usize, config: MorrisConfig, f: F) -> MorrisResult
    where
        F: Fn(&[f64]) -> f64,
    {
        let mut rng = ChaCha12Rng::seed_from_u64(42);
        let trajectories = generate_trajectories(dim, config, &mut rng);

        let all_values: Vec<f64> = trajectories
            .iter()
            .flat_map(|traj| traj.points.iter())
            .map(|pt| f(pt))
            .collect();

        let ee_per_param =
            extract_elementary_effects(&all_values, &trajectories, dim, config.levels);

        let params: Vec<ParameterDef> = (0..dim)
            .map(|i| ParameterDef {
                name: format!("x{i}"),
                min: 0.0,
                max: 1.0,
                kind: ParameterKind::Continuous,
            })
            .collect();
        let space = ParameterSpace::new(params).unwrap();

        aggregate_morris(ee_per_param, &space, config)
    }

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

    #[test]
    fn ee_constant_function() {
        let result = synthetic_screening(3, default_config(), |_| 42.0);
        for p in &result.parameters {
            assert!(p.mu.abs() < 1e-12, "mu should be 0, got {}", p.mu);
            assert!(p.mu_star < 1e-12, "mu_star should be 0, got {}", p.mu_star);
            assert!(p.sigma < 1e-12, "sigma should be 0, got {}", p.sigma);
            for &ee in &p.elementary_effects {
                assert!(ee.abs() < 1e-12, "EE should be 0, got {ee}");
            }
        }
    }

    #[test]
    fn ee_linear_function() {
        let result = synthetic_screening(2, default_config(), |x| 3.0 * x[0] + x[1]);
        let p0 = &result.parameters[0];
        let p1 = &result.parameters[1];

        for &ee in &p0.elementary_effects {
            assert!(
                (ee - 3.0).abs() < 1e-10,
                "EE for x0 should be 3.0, got {ee}"
            );
        }
        for &ee in &p1.elementary_effects {
            assert!(
                (ee - 1.0).abs() < 1e-10,
                "EE for x1 should be 1.0, got {ee}"
            );
        }

        assert!((p0.mu - 3.0).abs() < 1e-10);
        assert!((p0.mu_star - 3.0).abs() < 1e-10);
        assert!(p0.sigma < 1e-10);

        assert!((p1.mu - 1.0).abs() < 1e-10);
        assert!((p1.mu_star - 1.0).abs() < 1e-10);
        assert!(p1.sigma < 1e-10);
    }

    #[test]
    fn ee_quadratic_function() {
        let result = synthetic_screening(2, default_config(), |x| x[0] * x[0]);
        let p0 = &result.parameters[0];
        assert!(p0.mu_star > 0.0, "mu_star should be > 0 for quadratic");
        assert!(p0.sigma > 0.0, "sigma should be > 0 for nonlinear");
    }

    #[test]
    fn ee_interaction() {
        let result = synthetic_screening(2, default_config(), |x| x[0] * x[1]);
        for p in &result.parameters {
            assert!(
                p.sigma > 0.0,
                "sigma should be > 0 for interaction, param {}",
                p.name
            );
        }
    }

    #[test]
    fn mu_star_ranking() {
        let result = synthetic_screening(2, default_config(), |x| 3.0 * x[0] + x[1]);
        let rankings = result.rankings();
        assert_eq!(rankings[0].name, "x0", "coeff 3 should rank first");
        assert_eq!(rankings[1].name, "x1", "coeff 1 should rank second");
    }

    #[test]
    fn mu_star_nonnegative() {
        let result = synthetic_screening(3, default_config(), |x| x[0] * x[0] - 2.0 * x[1] + x[2]);
        for p in &result.parameters {
            assert!(
                p.mu_star >= 0.0,
                "mu_star must be >= 0, got {} for {}",
                p.mu_star,
                p.name
            );
        }
    }

    #[test]
    fn sigma_nonnegative() {
        let result = synthetic_screening(3, default_config(), |x| x[0] * x[1] * x[2]);
        for p in &result.parameters {
            assert!(
                p.sigma >= 0.0,
                "sigma must be >= 0, got {} for {}",
                p.sigma,
                p.name
            );
        }
    }

    #[test]
    fn sigma_uses_bessel_correction() {
        let config = MorrisConfig {
            trajectories: 2,
            levels: 4,
        };
        let result = synthetic_screening(2, config, |x| x[0] * x[0]);

        for p in &result.parameters {
            let ees = &p.elementary_effects;
            assert_eq!(ees.len(), 2, "should have 2 EEs with R=2");

            let n = ees.len() as f64;
            let mu = ees.iter().sum::<f64>() / n;
            let sigma_bessel =
                (ees.iter().map(|e| (e - mu).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            let sigma_pop = (ees.iter().map(|e| (e - mu).powi(2)).sum::<f64>() / n).sqrt();

            assert!(
                (p.sigma - sigma_bessel).abs() < 1e-15,
                "sigma should use Bessel's correction: got {}, expected {}",
                p.sigma,
                sigma_bessel
            );

            if sigma_bessel > 1e-15 {
                assert!(
                    (p.sigma - sigma_pop).abs() > 1e-15,
                    "sigma should differ from population std: both {}",
                    p.sigma
                );
            }
        }
    }
}
