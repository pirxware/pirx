//! Integration and property tests for Monte Carlo mode.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation
)]

use pirx_core::monte_carlo::{MonteCarloConfig, run_monte_carlo};
use pirx_hw::model::BufferConfig;
use pirx_testkit::{
    cultivation_hw, deterministic_distillation_hw, single_clifford, t_gate_chain, validated,
};
use proptest::prelude::*;

// ── Integration tests ────────────────────────────────────────────────────────

/// Same config → same MonteCarloResult.
#[test]
fn monte_carlo_deterministic() {
    let circuit = validated(t_gate_chain(5));
    let hw = cultivation_hw();
    let config = MonteCarloConfig {
        replicas: 10,
        base_seed: 42,
        max_cycles: None,
        threads: Some(1),
    };

    let r1 = run_monte_carlo(&circuit, &hw, config).unwrap();
    let r2 = run_monte_carlo(&circuit, &hw, config).unwrap();

    assert_eq!(r1.replicas.len(), r2.replicas.len());
    for (a, b) in r1.replicas.iter().zip(r2.replicas.iter()) {
        assert_eq!(a.seed, b.seed);
        assert_eq!(a.total_cycles, b.total_cycles);
        assert_eq!(a.stall_count, b.stall_count);
        assert_eq!(a.injection_errors, b.injection_errors);
    }
    assert!((r1.total_cycles.mean - r2.total_cycles.mean).abs() < f64::EPSILON);
}

/// Different base seeds → different means (with high probability).
#[test]
fn monte_carlo_different_seeds_different_results() {
    let circuit = validated(t_gate_chain(10));
    let hw = cultivation_hw();

    let r1 = run_monte_carlo(
        &circuit,
        &hw,
        MonteCarloConfig {
            replicas: 50,
            base_seed: 0,
            max_cycles: None,
            threads: Some(1),
        },
    )
    .unwrap();

    let r2 = run_monte_carlo(
        &circuit,
        &hw,
        MonteCarloConfig {
            replicas: 50,
            base_seed: 1_000_000,
            max_cycles: None,
            threads: Some(1),
        },
    )
    .unwrap();

    // With stochastic cultivation and 50 replicas each, the exact means will differ.
    // In the astronomically unlikely event they're identical, the test may flake.
    assert!(
        (r1.total_cycles.mean - r2.total_cycles.mean).abs() > 0.0
            || (r1.stall_count.mean - r2.stall_count.mean).abs() > 0.0,
        "different base seeds should produce different aggregate statistics"
    );
}

/// Single replica = equivalent to running Engine::run once.
#[test]
fn monte_carlo_single_replica() {
    let circuit = validated(t_gate_chain(3));
    let hw = cultivation_hw();
    let config = MonteCarloConfig {
        replicas: 1,
        base_seed: 42,
        max_cycles: None,
        threads: Some(1),
    };

    let result = run_monte_carlo(&circuit, &hw, config).unwrap();

    assert_eq!(result.replicas.len(), 1);
    let summary = &result.replicas[0];
    assert_eq!(summary.seed, 42);
    assert_eq!(result.total_cycles.mean, summary.total_cycles as f64);
    assert_eq!(result.total_cycles.stddev, 0.0);
}

/// Cultivation with many replicas: stddev of total_cycles > 0.
#[test]
fn monte_carlo_cultivation_variance() {
    let circuit = validated(t_gate_chain(5));
    let hw = cultivation_hw();
    let config = MonteCarloConfig {
        replicas: 100,
        base_seed: 0,
        max_cycles: None,
        threads: None,
    };

    let result = run_monte_carlo(&circuit, &hw, config).unwrap();

    assert!(
        result.total_cycles.stddev > 0.0,
        "cultivation is stochastic — stddev must be > 0 with 100 replicas"
    );
}

/// Deterministic distillation (abort_probability=0): all replicas produce the
/// same total_cycles → stddev ≈ 0.
#[test]
fn monte_carlo_distillation_no_abort_zero_variance() {
    let circuit = validated(single_clifford());
    let hw = deterministic_distillation_hw(1, 4, 0);
    let config = MonteCarloConfig {
        replicas: 20,
        base_seed: 0,
        max_cycles: None,
        threads: Some(1),
    };

    let result = run_monte_carlo(&circuit, &hw, config).unwrap();

    // All-Clifford circuit + deterministic factory → identical total_cycles.
    assert!(
        result.total_cycles.stddev < f64::EPSILON,
        "deterministic factory + Clifford circuit: stddev should be 0, got {}",
        result.total_cycles.stddev
    );
}

/// max_cycles causes truncation; truncated_count matches.
#[test]
fn monte_carlo_truncation_count() {
    let circuit = validated(t_gate_chain(10));
    let hw = cultivation_hw();
    let config = MonteCarloConfig {
        replicas: 20,
        base_seed: 0,
        max_cycles: Some(5),
        threads: Some(1),
    };

    let result = run_monte_carlo(&circuit, &hw, config).unwrap();

    // With max_cycles=5 and a T-gate chain needing factory production,
    // all replicas should be truncated.
    assert_eq!(
        result.truncated_count,
        result.replicas.len() as u32,
        "all replicas should be truncated with max_cycles=5"
    );
    assert!(result.replicas.iter().all(|s| s.truncated));
}

/// threads=1 gives same results as threads=4.
#[test]
fn monte_carlo_thread_count() {
    let circuit = validated(t_gate_chain(5));
    let hw = cultivation_hw();

    let base = MonteCarloConfig {
        replicas: 20,
        base_seed: 42,
        max_cycles: None,
        threads: Some(1),
    };

    let r1 = run_monte_carlo(&circuit, &hw, base).unwrap();
    let r4 = run_monte_carlo(
        &circuit,
        &hw,
        MonteCarloConfig {
            threads: Some(4),
            ..base
        },
    )
    .unwrap();

    // Same seeds → same per-replica summaries regardless of thread count.
    for (a, b) in r1.replicas.iter().zip(r4.replicas.iter()) {
        assert_eq!(a.seed, b.seed);
        assert_eq!(a.total_cycles, b.total_cycles);
        assert_eq!(a.stall_count, b.stall_count);
        assert_eq!(a.injection_errors, b.injection_errors);
    }
}

/// Clifford-only circuit: zero stalls, zero injection errors, zero buffer_full.
#[test]
fn monte_carlo_clifford_only() {
    let circuit = validated(single_clifford());
    let hw = cultivation_hw();
    let config = MonteCarloConfig {
        replicas: 10,
        base_seed: 0,
        max_cycles: None,
        threads: Some(1),
    };

    let result = run_monte_carlo(&circuit, &hw, config).unwrap();

    for summary in &result.replicas {
        assert_eq!(summary.stall_count, 0);
        assert_eq!(summary.injection_errors, 0);
        assert_eq!(summary.fixups_inserted, 0);
    }
    assert!((result.stall_count.mean).abs() < f64::EPSILON);
}

// ── Property tests ───────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// For any valid Monte Carlo run: min <= median <= max, stddev >= 0, mean in [min, max].
    #[test]
    fn monte_carlo_distribution_invariants(seed in 0u64..5_000) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(5));
        let mut hw = pirx_testkit::cultivation_hw();
        hw.buffer = BufferConfig { capacity: 4, preload: 2 };

        let config = MonteCarloConfig {
            replicas: 20,
            base_seed: seed,
            max_cycles: None,
            threads: Some(1),
        };

        let result = run_monte_carlo(&circuit, &hw, config).unwrap();

        for dist in [
            &result.total_cycles,
            &result.stall_count,
            &result.total_stall_cycles,
            &result.max_stall_cycles,
            &result.injection_errors,
            &result.fixups_inserted,
            &result.mean_factory_utilization,
            &result.buffer_full_events,
        ] {
            prop_assert!(dist.min <= dist.median, "min ({}) > median ({})", dist.min, dist.median);
            prop_assert!(dist.median <= dist.max, "median ({}) > max ({})", dist.median, dist.max);
            prop_assert!(dist.stddev >= 0.0, "stddev ({}) < 0", dist.stddev);
            prop_assert!(dist.mean >= dist.min - f64::EPSILON, "mean ({}) < min ({})", dist.mean, dist.min);
            prop_assert!(dist.mean <= dist.max + f64::EPSILON, "mean ({}) > max ({})", dist.mean, dist.max);
            prop_assert!(dist.p5 <= dist.p95, "p5 ({}) > p95 ({})", dist.p5, dist.p95);
            prop_assert!(dist.p25 <= dist.p75, "p25 ({}) > p75 ({})", dist.p25, dist.p75);
        }
    }
}
