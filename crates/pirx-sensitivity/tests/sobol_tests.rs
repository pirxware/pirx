//! Sobol variance-based sensitivity analysis integration and property tests.
//!
//! Integration tests run the full `sobol_analysis` pipeline through the
//! real engine with pirx-testkit circuits and hardware models.
//! Property tests verify mathematical invariants that hold for any valid seed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]

use pirx_sensitivity::{
    EvalConfig, OutputMetric, ParameterDef, ParameterKind, ParameterSpace, SobolConfig,
    sobol_analysis,
};
use pirx_testkit::{cultivation_hw, deterministic_distillation_hw, validated};
use proptest::prelude::*;

// -- Helpers ------------------------------------------------------------------

fn t_chain_circuit() -> pirx_ir::ValidatedCircuit {
    validated(pirx_testkit::t_gate_chain(5))
}

fn clifford_circuit() -> pirx_ir::ValidatedCircuit {
    validated(pirx_testkit::clifford_chain(10))
}

fn cultivation_space() -> ParameterSpace {
    ParameterSpace::new(vec![
        ParameterDef {
            name: "factory_count".into(),
            min: 2.0,
            max: 16.0,
            kind: ParameterKind::Integer,
        },
        ParameterDef {
            name: "buffer_capacity".into(),
            min: 4.0,
            max: 32.0,
            kind: ParameterKind::Integer,
        },
    ])
    .expect("valid space")
}

fn default_eval_config(seed: u64) -> EvalConfig {
    EvalConfig {
        mc_replicas: 1,
        base_seed: seed,
        max_cycles: Some(50_000),
        metric: OutputMetric::TotalCycles,
    }
}

fn small_sobol_config() -> SobolConfig {
    SobolConfig {
        n_samples: 256,
        confidence: 0.95,
        bootstrap_resamples: 200,
    }
}

// -- Integration tests --------------------------------------------------------

#[test]
fn sobol_cultivation_factory_count_dominant() {
    let circuit = t_chain_circuit();
    let hw = cultivation_hw();
    let space = cultivation_space();
    let eval_config = default_eval_config(42);
    let config = small_sobol_config();

    let result = sobol_analysis(&circuit, &hw, &space, &eval_config, config)
        .expect("analysis should succeed");

    assert_eq!(result.parameters.len(), 2);

    let factory = result
        .parameters
        .iter()
        .find(|p| p.name == "factory_count")
        .expect("factory_count missing");
    let buffer = result
        .parameters
        .iter()
        .find(|p| p.name == "buffer_capacity")
        .expect("buffer_capacity missing");

    assert!(
        factory.s1 > buffer.s1,
        "factory_count S1 ({}) should dominate buffer_capacity S1 ({})",
        factory.s1,
        buffer.s1,
    );
}

#[test]
fn sobol_deterministic() {
    let circuit = t_chain_circuit();
    let hw = cultivation_hw();
    let space = cultivation_space();
    let eval_config = default_eval_config(99);
    let config = small_sobol_config();

    let r1 =
        sobol_analysis(&circuit, &hw, &space, &eval_config, config).expect("run 1 should succeed");
    let r2 =
        sobol_analysis(&circuit, &hw, &space, &eval_config, config).expect("run 2 should succeed");

    assert_eq!(r1.evaluations, r2.evaluations);
    for (p1, p2) in r1.parameters.iter().zip(r2.parameters.iter()) {
        assert_eq!(p1.name, p2.name);
        assert_eq!(p1.s1, p2.s1, "S1 mismatch for {}", p1.name);
        assert_eq!(p1.st, p2.st, "ST mismatch for {}", p1.name);
        assert_eq!(p1.s1_ci, p2.s1_ci, "S1 CI mismatch for {}", p1.name);
        assert_eq!(p1.st_ci, p2.st_ci, "ST CI mismatch for {}", p1.name);
    }
}

#[test]
fn sobol_evaluation_count() {
    let circuit = t_chain_circuit();
    let hw = cultivation_hw();
    let space = cultivation_space();
    let eval_config = default_eval_config(7);

    for n_samples in [64, 128, 256] {
        let config = SobolConfig {
            n_samples,
            confidence: 0.95,
            bootstrap_resamples: 100,
        };
        let result = sobol_analysis(&circuit, &hw, &space, &eval_config, config)
            .expect("analysis should succeed");

        let dim = space.dim() as u64;
        let expected = u64::from(n_samples) * (dim + 2);
        assert_eq!(
            result.evaluations, expected,
            "N={n_samples}: expected {expected} evaluations, got {}",
            result.evaluations
        );
    }
}

#[test]
fn sobol_distillation_zero_variance() {
    let circuit = clifford_circuit();
    let hw = deterministic_distillation_hw(4, 8, 0);

    let space = ParameterSpace::new(vec![ParameterDef {
        name: "fixup_cost_cycles".into(),
        min: 1.0,
        max: 2.0,
        kind: ParameterKind::Integer,
    }])
    .expect("valid space");

    let eval_config = EvalConfig {
        mc_replicas: 1,
        base_seed: 42,
        max_cycles: Some(50_000),
        metric: OutputMetric::StallCount,
    };
    let config = SobolConfig {
        n_samples: 64,
        confidence: 0.95,
        bootstrap_resamples: 100,
    };

    let result =
        sobol_analysis(&circuit, &hw, &space, &eval_config, config).expect("should succeed");

    for p in &result.parameters {
        assert!(
            p.s1.abs() < f64::EPSILON,
            "S1({}) should be ~0 for zero-variance output, got {}",
            p.name,
            p.s1,
        );
    }
}

// -- Property tests -----------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    #[test]
    fn sobol_st_geq_s1_always(seed in any::<u64>()) {
        let circuit = t_chain_circuit();
        let hw = cultivation_hw();
        let space = cultivation_space();
        let eval_config = default_eval_config(seed);
        // N=128 for tighter estimates than minimum N=64
        let config = SobolConfig {
            n_samples: 128,
            confidence: 0.95,
            bootstrap_resamples: 100,
        };

        let result = sobol_analysis(&circuit, &hw, &space, &eval_config, config)
            .expect("analysis should succeed");

        for p in &result.parameters {
            prop_assert!(
                p.st >= p.s1 - 0.5,
                "ST ({}) must be >= S1 ({}) - 0.5 for param {} (seed={seed})",
                p.st,
                p.s1,
                p.name
            );
        }
    }

    #[test]
    fn sobol_indices_nonneg(seed in any::<u64>()) {
        let circuit = t_chain_circuit();
        let hw = cultivation_hw();
        let space = cultivation_space();
        let eval_config = default_eval_config(seed);
        let config = SobolConfig {
            n_samples: 128,
            confidence: 0.95,
            bootstrap_resamples: 100,
        };

        let result = sobol_analysis(&circuit, &hw, &space, &eval_config, config)
            .expect("analysis should succeed");

        for p in &result.parameters {
            prop_assert!(
                p.s1 >= -1.0,
                "S1 ({}) must be >= -1.0 for param {} (seed={seed})",
                p.s1,
                p.name
            );
            prop_assert!(
                p.st >= -0.5,
                "ST ({}) must be >= -0.5 for param {} (seed={seed})",
                p.st,
                p.name
            );
        }
    }

    #[test]
    fn sobol_ci_ordered(seed in any::<u64>()) {
        let circuit = t_chain_circuit();
        let hw = cultivation_hw();
        let space = cultivation_space();
        let eval_config = default_eval_config(seed);
        let config = SobolConfig {
            n_samples: 64,
            confidence: 0.95,
            bootstrap_resamples: 100,
        };

        let result = sobol_analysis(&circuit, &hw, &space, &eval_config, config)
            .expect("analysis should succeed");

        for p in &result.parameters {
            let (s1_lo, s1_hi) = p.s1_ci;
            let (st_lo, st_hi) = p.st_ci;
            prop_assert!(
                s1_lo <= s1_hi,
                "S1 CI lower ({s1_lo}) > upper ({s1_hi}) for param {} (seed={seed})",
                p.name
            );
            prop_assert!(
                st_lo <= st_hi,
                "ST CI lower ({st_lo}) > upper ({st_hi}) for param {} (seed={seed})",
                p.name
            );
        }
    }
}
