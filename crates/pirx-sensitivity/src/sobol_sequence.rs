//! Sobol quasi-random sequence generator using Joe-Kuo (2010) direction numbers.

use crate::error::SensitivityError;

const BITS: u32 = 32;
const NORM: f64 = 1.0 / (1_u64 << BITS) as f64;

/// Joe-Kuo direction numbers for dimensions 2..31.
/// Each entry: (degree s, polynomial coefficient a, initial direction numbers m[1..s]).
/// Source: <https://web.maths.unsw.edu.au/~fkuo/sobol/joe-kuo-old.1111>
const JK_DIRECTION_NUMBERS: &[(u32, u32, &[u32])] = &[
    (1, 0, &[1]),                        // dim 2
    (2, 1, &[1, 1]),                     // dim 3
    (3, 1, &[1, 1, 1]),                  // dim 4
    (3, 2, &[1, 3, 1]),                  // dim 5
    (4, 1, &[1, 1, 3, 3]),               // dim 6
    (4, 4, &[1, 3, 5, 13]),              // dim 7
    (5, 2, &[1, 1, 5, 5, 17]),           // dim 8
    (5, 4, &[1, 1, 5, 5, 5]),            // dim 9
    (5, 7, &[1, 1, 7, 11, 19]),          // dim 10
    (5, 11, &[1, 1, 5, 1, 1]),           // dim 11
    (5, 13, &[1, 1, 1, 3, 11]),          // dim 12
    (5, 14, &[1, 3, 5, 5, 31]),          // dim 13
    (6, 1, &[1, 3, 3, 9, 7, 49]),        // dim 14
    (6, 13, &[1, 1, 1, 15, 21, 21]),     // dim 15
    (6, 16, &[1, 3, 1, 13, 27, 49]),     // dim 16
    (6, 19, &[1, 1, 1, 15, 7, 5]),       // dim 17
    (6, 22, &[1, 3, 1, 3, 29, 1]),       // dim 18
    (6, 25, &[1, 1, 5, 7, 11, 15]),      // dim 19
    (7, 1, &[1, 1, 1, 1, 1, 1, 1]),      // dim 20
    (7, 4, &[1, 3, 7, 5, 9, 37, 75]),    // dim 21
    (7, 7, &[1, 3, 5, 13, 23, 53, 83]),  // dim 22
    (7, 8, &[1, 1, 1, 3, 7, 29, 67]),    // dim 23
    (7, 14, &[1, 1, 3, 7, 7, 49, 91]),   // dim 24
    (7, 19, &[1, 3, 5, 1, 1, 13, 13]),   // dim 25
    (7, 21, &[1, 3, 3, 3, 25, 41, 45]),  // dim 26
    (7, 28, &[1, 3, 7, 7, 27, 43, 119]), // dim 27
    (7, 31, &[1, 1, 7, 1, 25, 15, 71]),  // dim 28
    (7, 32, &[1, 3, 5, 5, 21, 37, 25]),  // dim 29
    (7, 37, &[1, 1, 3, 1, 23, 61, 109]), // dim 30
];

pub(crate) const MAX_DIM: usize = JK_DIRECTION_NUMBERS.len() + 1; // 30

/// Generate `n` Sobol quasi-random points in `dim` dimensions.
///
/// Returns `n` points, each a `Vec<f64>` of length `dim`, all values in [0, 1).
/// Uses Joe-Kuo direction numbers for dimensions 2..31.
/// Dimension 1 uses the van der Corput sequence.
///
/// `n` must be a power of 2 and `dim` must be in 1..=31.
pub fn sobol_sequence(n: usize, dim: usize) -> Result<Vec<Vec<f64>>, SensitivityError> {
    #[allow(clippy::cast_possible_truncation)]
    if n == 0 || !n.is_power_of_two() {
        return Err(SensitivityError::NotPowerOfTwo(n as u32));
    }
    if dim == 0 || dim > MAX_DIM {
        return Err(SensitivityError::DimensionMismatch {
            expected: MAX_DIM,
            actual: dim,
        });
    }

    let direction = init_direction_numbers(dim);
    let mut state = vec![0u32; dim];
    let mut points = Vec::with_capacity(n);

    // First point is the origin.
    points.push(vec![0.0_f64; dim]);

    for k in 1..n {
        let c = rightmost_zero_bit(k - 1);
        for j in 0..dim {
            #[allow(clippy::indexing_slicing)]
            {
                state[j] ^= direction[j][c];
            }
        }
        #[allow(clippy::indexing_slicing)]
        let point = state
            .iter()
            .map(|&s| f64::from(s) * NORM)
            .collect::<Vec<f64>>();
        points.push(point);
    }

    Ok(points)
}

/// Initialize direction numbers V[dim][bit] for all dimensions.
///
/// For dimension 1 (van der Corput): V[j] = 2^(31-j).
/// For dimensions 2..dim: use Joe-Kuo recurrence.
#[allow(clippy::indexing_slicing)]
fn init_direction_numbers(dim: usize) -> Vec<Vec<u32>> {
    let mut v = Vec::with_capacity(dim);

    // Dimension 1: van der Corput.
    let mut v0 = Vec::with_capacity(BITS as usize);
    for j in 0..BITS {
        v0.push(1u32 << (BITS - 1 - j));
    }
    v.push(v0);

    // Dimensions 2..dim via Joe-Kuo recurrence.
    for d in 1..dim {
        let (s, a, m) = JK_DIRECTION_NUMBERS[d - 1];
        let mut vd = vec![0u32; BITS as usize];

        // Initial direction numbers from the table, left-shifted.
        // s <= 7, so i always fits in u32.
        for i in 0..s as usize {
            #[allow(clippy::cast_possible_truncation)]
            let shift = i as u32;
            vd[i] = m[i] << (BITS - 1 - shift);
        }

        // Recurrence: V[i] = V[i-s] XOR (V[i-s] >> s) XOR sum of a-bits.
        for i in s as usize..BITS as usize {
            vd[i] = vd[i - s as usize] ^ (vd[i - s as usize] >> s);
            for k in 1..s {
                if (a >> (s - 1 - k)) & 1 == 1 {
                    vd[i] ^= vd[i - k as usize];
                }
            }
        }

        v.push(vd);
    }

    v
}

/// Position of the rightmost zero bit in `n` (0-indexed from LSB).
fn rightmost_zero_bit(n: usize) -> usize {
    (!n).trailing_zeros() as usize
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
    use super::*;

    #[test]
    fn sobol_seq_length() {
        for &n in &[1, 2, 4, 8, 16, 64, 256, 1024] {
            let pts = sobol_sequence(n, 3).unwrap();
            assert_eq!(pts.len(), n, "expected {n} points");
        }
    }

    #[test]
    fn sobol_seq_dimension() {
        for dim in 1..=MAX_DIM {
            let pts = sobol_sequence(16, dim).unwrap();
            for (i, pt) in pts.iter().enumerate() {
                assert_eq!(
                    pt.len(),
                    dim,
                    "point {i} should have {dim} dimensions, got {}",
                    pt.len()
                );
            }
        }
    }

    #[test]
    fn sobol_seq_unit_range() {
        let pts = sobol_sequence(4096, MAX_DIM).unwrap();
        for (i, pt) in pts.iter().enumerate() {
            for (j, &val) in pt.iter().enumerate() {
                assert!(
                    (0.0..1.0).contains(&val),
                    "point[{i}][{j}] = {val} not in [0, 1)"
                );
            }
        }
    }

    #[test]
    fn sobol_seq_deterministic() {
        let a = sobol_sequence(256, 5).unwrap();
        let b = sobol_sequence(256, 5).unwrap();
        assert_eq!(a, b, "same N, dim must produce identical output");
    }

    #[test]
    fn sobol_seq_first_point_origin() {
        let pts = sobol_sequence(64, 10).unwrap();
        for (j, &val) in pts[0].iter().enumerate() {
            assert!(
                val.abs() < f64::EPSILON,
                "first point dim {j} should be 0, got {val}"
            );
        }
    }

    #[test]
    fn sobol_seq_uniformity() {
        let n = 4096;
        let dim = 5;
        let pts = sobol_sequence(n, dim).unwrap();
        let bins = 10;
        let expected_per_bin = n as f64 / bins as f64;

        for d in 0..dim {
            let mut counts = vec![0usize; bins];
            for pt in &pts {
                let bin = (pt[d] * bins as f64).floor() as usize;
                let bin = bin.min(bins - 1);
                counts[bin] += 1;
            }

            let chi_sq: f64 = counts
                .iter()
                .map(|&c| {
                    let diff = c as f64 - expected_per_bin;
                    diff * diff / expected_per_bin
                })
                .sum();

            // Chi-squared critical value for df=9, p=0.01 is 21.67.
            assert!(
                chi_sq < 21.67,
                "dim {d} fails uniformity: chi_sq={chi_sq:.2} (threshold 21.67), counts={counts:?}"
            );
        }
    }

    #[test]
    fn sobol_seq_power_of_two_only() {
        for &bad_n in &[0, 3, 5, 6, 7, 9, 10, 15, 17, 100, 1000] {
            let result = sobol_sequence(bad_n, 3);
            assert!(
                matches!(result, Err(SensitivityError::NotPowerOfTwo(_))),
                "n={bad_n} should fail with NotPowerOfTwo, got {result:?}"
            );
        }
    }

    #[test]
    fn sobol_seq_dim_zero_rejected() {
        let result = sobol_sequence(16, 0);
        assert!(
            matches!(result, Err(SensitivityError::DimensionMismatch { .. })),
            "dim=0 should fail"
        );
    }

    #[test]
    fn sobol_seq_dim_too_large_rejected() {
        let result = sobol_sequence(16, MAX_DIM + 1);
        assert!(
            matches!(result, Err(SensitivityError::DimensionMismatch { .. })),
            "dim={} should fail",
            MAX_DIM + 1
        );
    }

    #[test]
    fn sobol_seq_no_duplicate_points() {
        let pts = sobol_sequence(256, 4).unwrap();
        for i in 0..pts.len() {
            for j in (i + 1)..pts.len() {
                assert_ne!(pts[i], pts[j], "points {i} and {j} are duplicates");
            }
        }
    }

    #[test]
    fn sobol_seq_n_1_single_origin() {
        let pts = sobol_sequence(1, 5).unwrap();
        assert_eq!(pts.len(), 1);
        assert!(pts[0].iter().all(|&v| v.abs() < f64::EPSILON));
    }
}
