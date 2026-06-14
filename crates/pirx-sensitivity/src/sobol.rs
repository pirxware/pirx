//! Sobol variance-based sensitivity indices.

#![allow(dead_code)]

use pirx_hw::model::HardwareModel;
use pirx_ir::ValidatedCircuit;
use rayon::prelude::*;

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
    pub fn validate(self) -> Result<(), SensitivityError> {
        if !self.n_samples.is_power_of_two() {
            return Err(SensitivityError::NotPowerOfTwo(self.n_samples));
        }
        if self.n_samples < 64 {
            return Err(SensitivityError::NotPowerOfTwo(self.n_samples));
        }
        if !(0.0..1.0).contains(&self.confidence) {
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
/// 1. Generate 2N quasi-random points via Sobol sequence
/// 2. Split: A = first N rows, B = last N rows
/// 3. For each param i: AB_i = copy of A with column i from B
#[allow(clippy::indexing_slicing)]
pub(crate) fn build_saltelli_matrices(
    n: usize,
    dim: usize,
) -> Result<SaltelliMatrices, SensitivityError> {
    let raw = sobol_sequence(2 * n, dim)?;
    let a: Vec<Vec<f64>> = raw[..n].to_vec();
    let b: Vec<Vec<f64>> = raw[n..].to_vec();
    let ab: Vec<Vec<Vec<f64>>> = (0..dim)
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
            matches!(err, SensitivityError::NotPowerOfTwo(32)),
            "expected NotPowerOfTwo(32), got {err:?}"
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
        for &bad in &[-0.1, 1.0, 1.5, f64::NAN] {
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
}
