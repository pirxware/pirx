//! Sobol variance-based sensitivity indices.

use pirx_hw::model::HardwareModel;
use pirx_ir::ValidatedCircuit;
use rand::{Rng as _, SeedableRng as _};
use rand_chacha::ChaCha12Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    config::SobolConfig,
    error::SensitivityError,
    parameter::ParameterSpace,
    sample::{EvalConfig, evaluate_point},
    sobol_sequence::sobol_sequence,
};

/// A matrix: N rows × P columns (unit-hypercube sample points).
type Matrix = Vec<Vec<f64>>;

/// Saltelli's A, B, and AB_i matrices.
type SaltelliMatrices = (Matrix, Matrix, Vec<Matrix>);

/// Evaluated output vectors for A, B, and each AB_i.
type EvaluatedOutputs = (Vec<f64>, Vec<f64>, Vec<Vec<f64>>);

impl SobolConfig {
    /// Validate Sobol configuration parameters.
    pub fn validate(&self) -> Result<(), SensitivityError> {
        if !self.n_samples.is_power_of_two() {
            return Err(SensitivityError::NotPowerOfTwo(self.n_samples));
        }
        if self.n_samples < 64 {
            return Err(SensitivityError::InsufficientSamples(self.n_samples));
        }
        if !(self.confidence > 0.0 && self.confidence < 1.0) {
            return Err(SensitivityError::ConfigParse(format!(
                "confidence must be in (0, 1), got {}",
                self.confidence
            )));
        }
        if self.bootstrap_resamples < 100 {
            return Err(SensitivityError::ConfigParse(format!(
                "bootstrap_resamples must be \u{2265} 100, got {}",
                self.bootstrap_resamples
            )));
        }
        Ok(())
    }
}

/// Construct A, B, and AB_i matrices for Saltelli's method.
///
/// Strategy selection:
/// - dim ≤ MAX_DIM/2 (currently 15): **column-split** — N points in 2P Sobol
///   dimensions, split by columns. Better quasi-independence between A and B.
/// - dim > MAX_DIM/2 but ≤ MAX_DIM (16–30): **row-split** — 2N points in P
///   Sobol dimensions, split by rows. Saltelli's original formulation (2002).
/// - dim > MAX_DIM: error.
#[allow(clippy::indexing_slicing)]
pub(crate) fn build_saltelli_matrices(
    n: usize,
    dim: usize,
) -> Result<SaltelliMatrices, SensitivityError> {
    use crate::sobol_sequence::MAX_DIM;

    if dim > MAX_DIM {
        return Err(SensitivityError::TooManyParameters {
            max: MAX_DIM,
            actual: dim,
        });
    }

    let (a, b) = if 2 * dim <= MAX_DIM {
        // Column-split: N points in 2P dimensions.
        let raw = sobol_sequence(n, 2 * dim)?;
        let a: Matrix = raw.iter().map(|row| row[..dim].to_vec()).collect();
        let b: Matrix = raw.iter().map(|row| row[dim..].to_vec()).collect();
        (a, b)
    } else {
        // Row-split: A from Sobol (good space-filling), B from Latin
        // Hypercube Sampling (stratified uniform, independent of A).
        // The Jansen estimator requires A and B to be independent samples.
        // Plain Sobol row-split fails because blocks of the same sequence
        // have structural correlation. LHS provides good uniformity with
        // guaranteed independence from A.
        let a: Matrix = sobol_sequence(n, dim)?;
        let b: Matrix = latin_hypercube(n, dim);
        (a, b)
    };

    // AB_i construction is identical for both strategies.
    let ab: Vec<Matrix> = (0..dim)
        .map(|i| {
            (0..n)
                .map(|j| {
                    let mut row = a[j].clone();
                    row[i] = b[j][i];
                    row
                })
                .collect()
        })
        .collect();

    Ok((a, b, ab))
}

/// Generate an N×dim Latin Hypercube Sample (stratified random).
///
/// For each dimension, [0,1] is divided into N equal strata, one point is
/// placed uniformly within each stratum, then the strata are shuffled
/// independently per dimension. RNG derived deterministically from (n, dim).
#[allow(clippy::cast_precision_loss, clippy::indexing_slicing)]
fn latin_hypercube(n: usize, dim: usize) -> Matrix {
    let seed = (n as u64)
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(dim as u64);
    let mut rng = ChaCha12Rng::seed_from_u64(seed);

    let columns: Vec<Vec<f64>> = (0..dim)
        .map(|_| {
            let mut col: Vec<f64> = (0..n)
                .map(|i| {
                    let lo = i as f64 / n as f64;
                    let hi = (i + 1) as f64 / n as f64;
                    lo + rng.random::<f64>() * (hi - lo)
                })
                .collect();
            // Fisher-Yates shuffle
            for i in (1..n).rev() {
                let j = rng.random_range(0..=i);
                col.swap(i, j);
            }
            col
        })
        .collect();

    (0..n)
        .map(|j| columns.iter().map(|col| col[j]).collect())
        .collect()
}

/// Flatten A, B, AB_0..AB_{P-1} into one batch, evaluate all in parallel,
/// and partition results back.
///
/// Total evaluations: N × (P + 2).
#[allow(clippy::indexing_slicing)]
pub(crate) fn evaluate_all(
    a: &[Vec<f64>],
    b: &[Vec<f64>],
    ab: &[Vec<Vec<f64>>],
    circuit: &ValidatedCircuit,
    base_hw: &HardwareModel,
    space: &ParameterSpace,
    eval_config: &EvalConfig,
) -> Result<EvaluatedOutputs, SensitivityError> {
    let n = a.len();
    let dim = space.dim();

    let mut all_points: Vec<&[f64]> = Vec::with_capacity(n * (dim + 2));
    for row in a {
        all_points.push(row);
    }
    for row in b {
        all_points.push(row);
    }
    for ab_i in ab {
        for row in ab_i {
            all_points.push(row);
        }
    }

    let all_values: Vec<f64> = all_points
        .par_iter()
        .enumerate()
        .map(|(idx, pt)| evaluate_point(circuit, base_hw, space, pt, idx, eval_config))
        .collect::<Result<_, _>>()?;

    let f_a = all_values[..n].to_vec();
    let f_b = all_values[n..2 * n].to_vec();
    let f_ab: Vec<Vec<f64>> = (0..dim)
        .map(|i| {
            let start = (2 + i) * n;
            all_values[start..start + n].to_vec()
        })
        .collect();
    Ok((f_a, f_b, f_ab))
}

/// Compute Var(f_A ∪ f_B) without allocating a merged vector.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn compute_variance(f_a: &[f64], f_b: &[f64]) -> f64 {
    let n_total = (f_a.len() + f_b.len()) as f64;
    let sum: f64 = f_a.iter().chain(f_b.iter()).sum();
    let mean = sum / n_total;
    f_a.iter()
        .chain(f_b.iter())
        .map(|&v| (v - mean).powi(2))
        .sum::<f64>()
        / n_total
}

/// Compute (S₁ᵢ, Sₜᵢ) for each parameter from pre-computed function values.
///
/// Saltelli (2010) estimator for S₁:
///   S₁ᵢ = [(1/N) Σ f_B[j] × (f_ABi[j] - f_A[j])] / Var(Y)
/// Jansen (1999) estimator for Sₜ:
///   Sₜᵢ = [(1/2N) Σ (f_A[j] - f_ABi[j])²] / Var(Y)
#[allow(clippy::cast_precision_loss, clippy::indexing_slicing)]
pub(crate) fn compute_indices(
    f_a: &[f64],
    f_b: &[f64],
    f_ab: &[Vec<f64>],
    dim: usize,
) -> Vec<(f64, f64)> {
    let n = f_a.len() as f64;
    let var_y = compute_variance(f_a, f_b);

    if var_y < f64::EPSILON {
        return vec![(0.0, 0.0); dim];
    }

    (0..dim)
        .map(|i| {
            let v_i: f64 = f_b
                .iter()
                .zip(f_ab[i].iter())
                .zip(f_a.iter())
                .map(|((&fb, &fab), &fa)| fb * (fab - fa))
                .sum::<f64>()
                / n;
            let s1 = v_i / var_y;

            let v_ti: f64 = f_a
                .iter()
                .zip(f_ab[i].iter())
                .map(|(&fa, &fab)| (fa - fab).powi(2))
                .sum::<f64>()
                / (2.0 * n);
            let st = v_ti / var_y;

            (s1, st)
        })
        .collect()
}

/// Bootstrap confidence intervals for all parameters.
///
/// Paired resampling: same index set for `f_a`, `f_b`, and all `f_ab[p]`
/// to preserve the correlation structure of Saltelli's design.
///
/// Returns `Vec<((s1_lo, s1_hi), (st_lo, st_hi))>` — one entry per parameter.
#[allow(clippy::cast_precision_loss, clippy::indexing_slicing)]
pub(crate) fn bootstrap_ci(
    f_a: &[f64],
    f_b: &[f64],
    f_ab: &[Vec<f64>],
    dim: usize,
    config: &SobolConfig,
    rng: &mut ChaCha12Rng,
) -> Vec<((f64, f64), (f64, f64))> {
    let n = f_a.len();
    let r = config.bootstrap_resamples as usize;
    let alpha = 1.0 - config.confidence;

    let mut s1_samples: Vec<Vec<f64>> = vec![Vec::with_capacity(r); dim];
    let mut st_samples: Vec<Vec<f64>> = vec![Vec::with_capacity(r); dim];

    for _ in 0..r {
        let indices: Vec<usize> = (0..n).map(|_| rng.random_range(0..n)).collect();
        let ra: Vec<f64> = indices.iter().map(|&i| f_a[i]).collect();
        let rb: Vec<f64> = indices.iter().map(|&i| f_b[i]).collect();
        let rab: Vec<Vec<f64>> = (0..dim)
            .map(|p| indices.iter().map(|&i| f_ab[p][i]).collect())
            .collect();

        for (p, (s1, st)) in compute_indices(&ra, &rb, &rab, dim).into_iter().enumerate() {
            s1_samples[p].push(s1);
            st_samples[p].push(st);
        }
    }

    (0..dim)
        .map(|p| {
            (
                percentile_ci(&mut s1_samples[p], alpha),
                percentile_ci(&mut st_samples[p], alpha),
            )
        })
        .collect()
}

/// Percentile-based confidence interval from bootstrap samples.
#[allow(clippy::indexing_slicing)]
fn percentile_ci(samples: &mut [f64], alpha: f64) -> (f64, f64) {
    samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = samples.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let lo = ((alpha / 2.0) * n as f64).floor() as usize;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let hi = ((1.0 - alpha / 2.0) * n as f64).ceil() as usize;
    (samples[lo], samples[hi.min(n - 1)])
}

/// Results of a Sobol variance-based sensitivity analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use = "Sobol analysis result is discarded — this ran thousands of engine evaluations"]
pub struct SobolResult {
    pub parameters: Vec<SobolParameterResult>,
    /// Sum of all first-order indices (close to 1.0 for additive models).
    pub s1_sum: f64,
    /// Total output variance Var(Y).
    pub output_variance: f64,
    /// Total number of model evaluations.
    pub evaluations: u64,
    pub config: SobolConfig,
}

impl SobolResult {
    /// Parameters ranked by descending S₁ (most influential first-order effect first).
    #[must_use]
    pub fn rankings(&self) -> Vec<&SobolParameterResult> {
        let mut r: Vec<_> = self.parameters.iter().collect();
        r.sort_by(|a, b| b.s1.partial_cmp(&a.s1).unwrap_or(std::cmp::Ordering::Equal));
        r
    }

    /// Parameters ranked by descending Sₜ (most influential total effect first).
    #[must_use]
    pub fn rankings_total(&self) -> Vec<&SobolParameterResult> {
        let mut r: Vec<_> = self.parameters.iter().collect();
        r.sort_by(|a, b| b.st.partial_cmp(&a.st).unwrap_or(std::cmp::Ordering::Equal));
        r
    }
}

/// Sobol sensitivity indices for a single parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SobolParameterResult {
    pub name: String,
    /// First-order sensitivity index.
    pub s1: f64,
    /// Total-order sensitivity index.
    pub st: f64,
    /// Bootstrap confidence interval for S₁.
    pub s1_ci: (f64, f64),
    /// Bootstrap confidence interval for Sₜ.
    pub st_ci: (f64, f64),
    /// Interaction effect: Sₜ - S₁.
    pub interaction: f64,
}

/// Run a complete Sobol variance-based sensitivity analysis.
///
/// Supports up to 30 parameters (the maximum dimension covered by Joe-Kuo
/// direction numbers). For ≤15 parameters, uses column-split Saltelli
/// matrices; for 16–30, uses row-split.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::indexing_slicing
)]
pub fn sobol_analysis(
    circuit: &ValidatedCircuit,
    base_hw: &HardwareModel,
    space: &ParameterSpace,
    eval_config: &EvalConfig,
    config: SobolConfig,
) -> Result<SobolResult, SensitivityError> {
    config.validate()?;
    if space.dim() == 0 {
        return Err(SensitivityError::EmptyParameterSpace);
    }
    if space.dim() > crate::sobol_sequence::MAX_DIM {
        return Err(SensitivityError::TooManyParameters {
            max: crate::sobol_sequence::MAX_DIM,
            actual: space.dim(),
        });
    }

    let n = config.n_samples as usize;
    let dim = space.dim();

    let (a, b, ab) = build_saltelli_matrices(n, dim)?;
    let (f_a, f_b, f_ab) = evaluate_all(&a, &b, &ab, circuit, base_hw, space, eval_config)?;
    let indices = compute_indices(&f_a, &f_b, &f_ab, dim);

    let mut bootstrap_rng = ChaCha12Rng::seed_from_u64(
        eval_config
            .base_seed
            .wrapping_mul(0x9E3779B97F4A7C15)
            .wrapping_add(1),
    );
    let cis = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config, &mut bootstrap_rng);

    let output_variance = compute_variance(&f_a, &f_b);
    let parameters: Vec<SobolParameterResult> = (0..dim)
        .map(|i| {
            let (s1, st) = indices[i];
            let (s1_ci, st_ci) = cis[i];
            SobolParameterResult {
                name: space.params()[i].name.clone(),
                s1,
                st,
                s1_ci,
                st_ci,
                interaction: st - s1,
            }
        })
        .collect();

    Ok(SobolResult {
        s1_sum: parameters.iter().map(|p| p.s1).sum(),
        parameters,
        output_variance,
        evaluations: n as u64 * (dim as u64 + 2),
        config,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]
mod tests {
    use rand::SeedableRng;

    use super::*;

    #[test]
    fn matrices_dimensions() {
        let n = 64;
        let dim = 5;
        let (a, b, ab) = build_saltelli_matrices(n, dim).unwrap();

        assert_eq!(a.len(), n, "A should have N rows");
        assert_eq!(b.len(), n, "B should have N rows");
        for (i, row) in a.iter().enumerate() {
            assert_eq!(row.len(), dim, "A row {i} should have P columns");
        }
        for (i, row) in b.iter().enumerate() {
            assert_eq!(row.len(), dim, "B row {i} should have P columns");
        }
        assert_eq!(ab.len(), dim, "AB should have P matrices");
        for (i, ab_i) in ab.iter().enumerate() {
            assert_eq!(ab_i.len(), n, "AB_{i} should have N rows");
            for (j, row) in ab_i.iter().enumerate() {
                assert_eq!(row.len(), dim, "AB_{i} row {j} should have P columns");
            }
        }
    }

    #[test]
    fn matrices_ab_column_replacement() {
        let n = 128;
        let dim = 4;
        let (a, b, ab) = build_saltelli_matrices(n, dim).unwrap();

        for i in 0..dim {
            for j in 0..n {
                assert!(
                    (ab[i][j][i] - b[j][i]).abs() < f64::EPSILON,
                    "AB_{i} row {j} column {i} should match B"
                );
                for k in 0..dim {
                    if k != i {
                        assert!(
                            (ab[i][j][k] - a[j][k]).abs() < f64::EPSILON,
                            "AB_{i} row {j} column {k} should match A"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn config_rejects_non_power_of_two() {
        let config = SobolConfig {
            n_samples: 1000,
            confidence: 0.95,
            bootstrap_resamples: 1000,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, SensitivityError::NotPowerOfTwo(1000)),
            "expected NotPowerOfTwo(1000), got {err:?}"
        );
    }

    #[test]
    fn config_rejects_too_few_samples() {
        let config = SobolConfig {
            n_samples: 32,
            confidence: 0.95,
            bootstrap_resamples: 1000,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, SensitivityError::InsufficientSamples(32)),
            "expected InsufficientSamples(32), got {err:?}"
        );
    }

    #[test]
    fn config_rejects_low_bootstrap() {
        let config = SobolConfig {
            n_samples: 1024,
            confidence: 0.95,
            bootstrap_resamples: 10,
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, SensitivityError::ConfigParse(_)),
            "expected ConfigParse, got {err:?}"
        );
    }

    #[test]
    fn config_rejects_bad_confidence() {
        for &bad in &[0.0, -0.1, 1.0, 1.5, f64::NAN] {
            let config = SobolConfig {
                n_samples: 1024,
                confidence: bad,
                bootstrap_resamples: 1000,
            };
            assert!(
                config.validate().is_err(),
                "confidence={bad} should be rejected"
            );
        }
    }

    #[test]
    fn config_accepts_valid() {
        let config = SobolConfig {
            n_samples: 1024,
            confidence: 0.95,
            bootstrap_resamples: 1000,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn config_accepts_boundary_values() {
        let config = SobolConfig {
            n_samples: 64,
            confidence: 0.5,
            bootstrap_resamples: 100,
        };
        assert!(config.validate().is_ok());
    }

    /// Evaluate a synthetic function on Saltelli matrices and compute indices.
    fn synthetic_sobol<F>(n: usize, dim: usize, f: F) -> Vec<(f64, f64)>
    where
        F: Fn(&[f64]) -> f64,
    {
        let (a, b, ab) = build_saltelli_matrices(n, dim).unwrap();
        let f_a: Vec<f64> = a.iter().map(|row| f(row)).collect();
        let f_b: Vec<f64> = b.iter().map(|row| f(row)).collect();
        let f_ab: Vec<Vec<f64>> = ab
            .iter()
            .map(|ab_i| ab_i.iter().map(|row| f(row)).collect())
            .collect();
        compute_indices(&f_a, &f_b, &f_ab, dim)
    }

    #[test]
    fn sobol_linear_additive() {
        let indices = synthetic_sobol(4096, 2, |x| 3.0 * x[0] + x[1]);

        let (s1_x0, st_x0) = indices[0];
        let (s1_x1, st_x1) = indices[1];

        assert!(
            (s1_x0 - 0.9).abs() < 0.05,
            "S1(x0) should be ~0.9, got {s1_x0}"
        );
        assert!(
            (s1_x1 - 0.1).abs() < 0.05,
            "S1(x1) should be ~0.1, got {s1_x1}"
        );
        assert!(
            (st_x0 - s1_x0).abs() < 0.05,
            "ST(x0) should be ~S1(x0) for additive model, got ST={st_x0}, S1={s1_x0}"
        );
        assert!(
            (st_x1 - s1_x1).abs() < 0.05,
            "ST(x1) should be ~S1(x1) for additive model, got ST={st_x1}, S1={s1_x1}"
        );
    }

    #[test]
    fn sobol_single_variable() {
        let indices = synthetic_sobol(4096, 2, |x| x[0]);

        let (s1_x0, st_x0) = indices[0];
        let (s1_x1, _st_x1) = indices[1];

        assert!(
            (s1_x0 - 1.0).abs() < 0.05,
            "S1(x0) should be ~1.0, got {s1_x0}"
        );
        assert!(s1_x1.abs() < 0.05, "S1(x1) should be ~0.0, got {s1_x1}");
        assert!(
            (st_x0 - 1.0).abs() < 0.05,
            "ST(x0) should be ~1.0, got {st_x0}"
        );
    }

    #[test]
    fn sobol_constant() {
        let indices = synthetic_sobol(4096, 3, |_| 5.0);

        for (i, &(s1, st)) in indices.iter().enumerate() {
            assert!(s1.abs() < f64::EPSILON, "S1(x{i}) should be 0, got {s1}");
            assert!(st.abs() < f64::EPSILON, "ST(x{i}) should be 0, got {st}");
        }
    }

    #[test]
    fn sobol_sum_s1_close_to_one() {
        let indices = synthetic_sobol(4096, 3, |x| 2.0 * x[0] + x[1] + 0.5 * x[2]);

        let s1_sum: f64 = indices.iter().map(|(s1, _)| s1).sum();
        assert!(
            (0.9..=1.1).contains(&s1_sum),
            "sum of S1 should be in [0.9, 1.1] for additive model, got {s1_sum}"
        );
    }

    #[test]
    fn sobol_st_geq_s1() {
        let indices = synthetic_sobol(4096, 3, |x| x[0] * x[1] + x[2]);

        for (i, &(s1, st)) in indices.iter().enumerate() {
            assert!(
                st >= s1 - 0.05,
                "ST(x{i}) should be >= S1(x{i}) - 0.05: ST={st}, S1={s1}"
            );
        }
    }

    #[test]
    fn variance_computation_no_alloc() {
        let f_a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let f_b = vec![6.0, 7.0, 8.0, 9.0, 10.0];

        let var = compute_variance(&f_a, &f_b);

        let mut merged = f_a.clone();
        merged.extend_from_slice(&f_b);
        let n = merged.len() as f64;
        let mean = merged.iter().sum::<f64>() / n;
        let naive_var = merged.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / n;

        assert!(
            (var - naive_var).abs() < 1e-12,
            "compute_variance={var} should match naive={naive_var}"
        );
    }

    // --- Ishigami validation ---

    /// Ishigami function: f(x₁, x₂, x₃) = sin(x₁) + 7·sin²(x₂) + 0.1·x₃⁴·sin(x₁)
    fn ishigami(x: &[f64]) -> f64 {
        let (x1, x2, x3) = (x[0], x[1], x[2]);
        x1.sin() + 7.0 * x2.sin().powi(2) + 0.1 * x3.powi(4) * x1.sin()
    }

    #[test]
    fn ishigami_analytical_indices() {
        let n = 4096;
        let dim = 3;
        let pi = std::f64::consts::PI;

        let (a, b, ab) = build_saltelli_matrices(n, dim).unwrap();

        let map_to_pi =
            |row: &[f64]| -> Vec<f64> { row.iter().map(|&u| -pi + 2.0 * pi * u).collect() };

        let f_a: Vec<f64> = a.iter().map(|row| ishigami(&map_to_pi(row))).collect();
        let f_b: Vec<f64> = b.iter().map(|row| ishigami(&map_to_pi(row))).collect();
        let f_ab: Vec<Vec<f64>> = ab
            .iter()
            .map(|ab_i| ab_i.iter().map(|row| ishigami(&map_to_pi(row))).collect())
            .collect();

        let indices = compute_indices(&f_a, &f_b, &f_ab, dim);

        // Analytical: S₁(x₁) ≈ 0.3139, S₁(x₂) ≈ 0.4424, S₁(x₃) = 0
        let (s1_x1, st_x1) = indices[0];
        let (s1_x2, st_x2) = indices[1];
        let (s1_x3, st_x3) = indices[2];

        assert!(
            (s1_x1 - 0.3139).abs() < 0.05,
            "S1(x1) should be ~0.3139, got {s1_x1}"
        );
        assert!(
            (s1_x2 - 0.4424).abs() < 0.05,
            "S1(x2) should be ~0.4424, got {s1_x2}"
        );
        assert!(s1_x3.abs() < 0.05, "S1(x3) should be ~0, got {s1_x3}");

        // Analytical: Sₜ(x₁) ≈ 0.5576, Sₜ(x₂) ≈ 0.4424, Sₜ(x₃) ≈ 0.2437
        assert!(
            (st_x1 - 0.5576).abs() < 0.06,
            "ST(x1) should be ~0.5576, got {st_x1}"
        );
        assert!(
            (st_x2 - 0.4424).abs() < 0.05,
            "ST(x2) should be ~0.4424, got {st_x2}"
        );
        assert!(
            (st_x3 - 0.2437).abs() < 0.05,
            "ST(x3) should be ~0.2437, got {st_x3}"
        );
    }

    // --- Bootstrap CI tests ---

    fn linear_sobol_outputs(n: usize) -> (Vec<f64>, Vec<f64>, Vec<Vec<f64>>) {
        let dim = 2;
        let f = |x: &[f64]| 3.0 * x[0] + x[1];
        let (a, b, ab) = build_saltelli_matrices(n, dim).unwrap();
        let f_a: Vec<f64> = a.iter().map(|row| f(row)).collect();
        let f_b: Vec<f64> = b.iter().map(|row| f(row)).collect();
        let f_ab: Vec<Vec<f64>> = ab
            .iter()
            .map(|ab_i| ab_i.iter().map(|row| f(row)).collect())
            .collect();
        (f_a, f_b, f_ab)
    }

    fn default_sobol_config() -> SobolConfig {
        SobolConfig {
            n_samples: 1024,
            confidence: 0.95,
            bootstrap_resamples: 1000,
        }
    }

    #[test]
    fn ci_contains_point_estimate() {
        let (f_a, f_b, f_ab) = linear_sobol_outputs(1024);
        let dim = 2;
        let config = default_sobol_config();
        let mut rng =
            ChaCha12Rng::seed_from_u64(42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));

        let point = compute_indices(&f_a, &f_b, &f_ab, dim);
        let cis = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config, &mut rng);

        for p in 0..dim {
            let (s1, st) = point[p];
            let ((s1_lo, s1_hi), (st_lo, st_hi)) = cis[p];
            assert!(
                s1_lo <= s1 && s1 <= s1_hi,
                "param {p}: S1={s1} not in [{s1_lo}, {s1_hi}]"
            );
            assert!(
                st_lo <= st && st <= st_hi,
                "param {p}: ST={st} not in [{st_lo}, {st_hi}]"
            );
        }
    }

    #[test]
    fn ci_width_decreases_with_n() {
        let dim = 2;
        let config = default_sobol_config();

        let (f_a_small, f_b_small, f_ab_small) = linear_sobol_outputs(256);
        let mut rng_small =
            ChaCha12Rng::seed_from_u64(42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));
        let ci_small = bootstrap_ci(
            &f_a_small,
            &f_b_small,
            &f_ab_small,
            dim,
            &config,
            &mut rng_small,
        );

        let (f_a_large, f_b_large, f_ab_large) = linear_sobol_outputs(2048);
        let mut rng_large =
            ChaCha12Rng::seed_from_u64(42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));
        let ci_large = bootstrap_ci(
            &f_a_large,
            &f_b_large,
            &f_ab_large,
            dim,
            &config,
            &mut rng_large,
        );

        for p in 0..dim {
            let width_small = ci_small[p].0.1 - ci_small[p].0.0;
            let width_large = ci_large[p].0.1 - ci_large[p].0.0;
            assert!(
                width_large < width_small,
                "param {p}: S1 CI should narrow with more samples: \
                 N=256 width={width_small:.4}, N=2048 width={width_large:.4}"
            );
        }
    }

    #[test]
    fn ci_95_wider_than_68() {
        let (f_a, f_b, f_ab) = linear_sobol_outputs(1024);
        let dim = 2;

        let config_95 = SobolConfig {
            confidence: 0.95,
            ..default_sobol_config()
        };
        let config_68 = SobolConfig {
            confidence: 0.68,
            ..default_sobol_config()
        };

        let mut rng_95 =
            ChaCha12Rng::seed_from_u64(42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));
        let mut rng_68 =
            ChaCha12Rng::seed_from_u64(42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));

        let ci_95 = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config_95, &mut rng_95);
        let ci_68 = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config_68, &mut rng_68);

        for p in 0..dim {
            let w95 = ci_95[p].0.1 - ci_95[p].0.0;
            let w68 = ci_68[p].0.1 - ci_68[p].0.0;
            assert!(
                w95 > w68,
                "param {p}: 95% CI should be wider than 68%: w95={w95:.4}, w68={w68:.4}"
            );
        }
    }

    #[test]
    fn ci_deterministic() {
        let (f_a, f_b, f_ab) = linear_sobol_outputs(1024);
        let dim = 2;
        let config = default_sobol_config();
        let seed = 42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1);

        let mut rng1 = ChaCha12Rng::seed_from_u64(seed);
        let ci1 = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config, &mut rng1);

        let mut rng2 = ChaCha12Rng::seed_from_u64(seed);
        let ci2 = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config, &mut rng2);

        for p in 0..dim {
            assert!(
                (ci1[p].0.0 - ci2[p].0.0).abs() < f64::EPSILON
                    && (ci1[p].0.1 - ci2[p].0.1).abs() < f64::EPSILON
                    && (ci1[p].1.0 - ci2[p].1.0).abs() < f64::EPSILON
                    && (ci1[p].1.1 - ci2[p].1.1).abs() < f64::EPSILON,
                "param {p}: CIs must be identical for same seed"
            );
        }
    }

    #[test]
    fn ci_ordered() {
        let (f_a, f_b, f_ab) = linear_sobol_outputs(1024);
        let dim = 2;
        let config = default_sobol_config();
        let mut rng =
            ChaCha12Rng::seed_from_u64(42_u64.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1));

        let cis = bootstrap_ci(&f_a, &f_b, &f_ab, dim, &config, &mut rng);

        for (p, &((s1_lo, s1_hi), (st_lo, st_hi))) in cis.iter().enumerate() {
            assert!(
                s1_lo <= s1_hi,
                "param {p}: S1 CI lower {s1_lo} > upper {s1_hi}"
            );
            assert!(
                st_lo <= st_hi,
                "param {p}: ST CI lower {st_lo} > upper {st_hi}"
            );
        }
    }

    // --- Dual-strategy (column-split / row-split) tests ---

    #[test]
    fn row_split_produces_valid_matrices() {
        let n = 64;
        let dim = 20; // 2*20 = 40 > 30, forces row-split
        let (a, b, ab) = build_saltelli_matrices(n, dim).unwrap();

        assert_eq!(a.len(), n, "A should have N rows");
        assert_eq!(b.len(), n, "B should have N rows");
        for (i, row) in a.iter().enumerate() {
            assert_eq!(row.len(), dim, "A row {i} should have P columns");
        }
        for (i, row) in b.iter().enumerate() {
            assert_eq!(row.len(), dim, "B row {i} should have P columns");
        }
        assert_eq!(ab.len(), dim, "AB should have P matrices");
        for (i, ab_i) in ab.iter().enumerate() {
            assert_eq!(ab_i.len(), n, "AB_{i} should have N rows");
            for (j, row) in ab_i.iter().enumerate() {
                assert_eq!(row.len(), dim, "AB_{i} row {j} should have P columns");
            }
        }

        // Verify AB_i column replacement property
        for i in 0..dim {
            for j in 0..n {
                assert!(
                    (ab[i][j][i] - b[j][i]).abs() < f64::EPSILON,
                    "AB_{i} row {j} column {i} should match B"
                );
                for k in 0..dim {
                    if k != i {
                        assert!(
                            (ab[i][j][k] - a[j][k]).abs() < f64::EPSILON,
                            "AB_{i} row {j} column {k} should match A"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn row_split_ishigami() {
        let n = 4096;
        let total_dim = 16; // triggers row-split (2*16 = 32 > 30)
        let pi = std::f64::consts::PI;

        let (a, b, ab) = build_saltelli_matrices(n, total_dim).unwrap();

        // Map first 3 dims to [-pi, pi], ignore the rest (constant output)
        let map_to_pi =
            |row: &[f64]| -> Vec<f64> { row.iter().map(|&u| -pi + 2.0 * pi * u).collect() };

        let f_a: Vec<f64> = a.iter().map(|row| ishigami(&map_to_pi(row))).collect();
        let f_b: Vec<f64> = b.iter().map(|row| ishigami(&map_to_pi(row))).collect();
        let f_ab: Vec<Vec<f64>> = ab
            .iter()
            .map(|ab_i| ab_i.iter().map(|row| ishigami(&map_to_pi(row))).collect())
            .collect();

        let indices = compute_indices(&f_a, &f_b, &f_ab, total_dim);

        // First 3 dims: same expectations as column-split Ishigami test
        let (s1_x1, st_x1) = indices[0];
        let (s1_x2, st_x2) = indices[1];
        let (s1_x3, st_x3) = indices[2];

        assert!(
            (s1_x1 - 0.3139).abs() < 0.06,
            "S1(x1) should be ~0.3139, got {s1_x1}"
        );
        assert!(
            (s1_x2 - 0.4424).abs() < 0.06,
            "S1(x2) should be ~0.4424, got {s1_x2}"
        );
        assert!(s1_x3.abs() < 0.06, "S1(x3) should be ~0, got {s1_x3}");

        assert!(
            (st_x1 - 0.5576).abs() < 0.08,
            "ST(x1) should be ~0.5576, got {st_x1}"
        );
        assert!(
            (st_x2 - 0.4424).abs() < 0.06,
            "ST(x2) should be ~0.4424, got {st_x2}"
        );
        assert!(
            (st_x3 - 0.2437).abs() < 0.06,
            "ST(x3) should be ~0.2437, got {st_x3}"
        );

        // Dummy dimensions (3..16) should have near-zero indices
        for (i, &(s1, st)) in indices.iter().enumerate().skip(3) {
            assert!(
                s1.abs() < 0.06,
                "S1(x{i}) should be ~0 for dummy dim, got {s1}"
            );
            assert!(
                st.abs() < 0.06,
                "ST(x{i}) should be ~0 for dummy dim, got {st}"
            );
        }
    }

    #[test]
    fn rejects_too_many_parameters() {
        let result = build_saltelli_matrices(64, 31);
        assert!(
            matches!(
                result,
                Err(SensitivityError::TooManyParameters {
                    max: 30,
                    actual: 31
                })
            ),
            "expected TooManyParameters {{ max: 30, actual: 31 }}, got {result:?}"
        );
    }

    #[test]
    fn column_split_used_for_small_dim() {
        // 2*5 = 10 <= 30: column-split
        let result = build_saltelli_matrices(64, 5);
        assert!(result.is_ok(), "dim=5 should use column-split: {result:?}");

        // 2*15 = 30 <= 30: column-split
        let result = build_saltelli_matrices(64, 15);
        assert!(result.is_ok(), "dim=15 should use column-split: {result:?}");
    }

    #[test]
    fn row_split_used_for_large_dim() {
        // 2*16 = 32 > 30: row-split
        let result = build_saltelli_matrices(64, 16);
        assert!(result.is_ok(), "dim=16 should use row-split: {result:?}");

        // dim=30 == MAX_DIM: row-split
        let result = build_saltelli_matrices(64, 30);
        assert!(result.is_ok(), "dim=30 should use row-split: {result:?}");
    }
}
