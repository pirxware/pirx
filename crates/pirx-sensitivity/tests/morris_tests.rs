//! Morris screening integration and property tests.
//!
//! Integration tests run the full `morris_screening` pipeline through the
//! real engine with pirx-testkit circuits and hardware models.
//! Property tests verify mathematical invariants that hold for any valid
//! seed, trajectory count, and level count.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]

use pirx_sensitivity::{
    EvalConfig, MorrisConfig, OutputMetric, ParameterDef, ParameterKind, ParameterSpace,
    morris_screening,
};
use pirx_testkit::{cultivation_hw, validated};
use proptest::prelude::*;

// ── Helpers ─────────────────────────────────────────────────────────────────

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

// ── Integration tests ───────────────────────────────────────────────────────

#[test]
fn morris_t_gate_chain_cultivation() {
    let circuit = t_chain_circuit();
    let hw = cultivation_hw();
    let space = cultivation_space();
    let eval_config = default_eval_config(42);
    let morris_config = MorrisConfig {
        trajectories: 4,
        levels: 4,
    };

    let result = morris_screening(&circuit, &hw, &space, &eval_config, morris_config)
        .expect("screening should succeed");

    assert_eq!(
        result.parameters.len(),
        2,
        "should have one result per parameter"
    );

    let factory_count_result = result
        .parameters
        .iter()
        .find(|p| p.name == "factory_count")
        .expect("factory_count result missing");
    assert!(
        factory_count_result.mu_star > 0.0,
        "factory_count should have nonzero influence on a T-gate chain: mu_star={}",
        factory_count_result.mu_star
    );

    let dim = space.dim() as u64;
    let expected_evals = u64::from(morris_config.trajectories) * (dim + 1);
    assert_eq!(
        result.evaluations, expected_evals,
        "evaluations should be R * (P + 1)"
    );
}

#[test]
fn morris_deterministic() {
    let circuit = t_chain_circuit();
    let hw = cultivation_hw();
    let space = cultivation_space();
    let morris_config = MorrisConfig {
        trajectories: 3,
        levels: 4,
    };

    let eval_config = default_eval_config(99);

    let r1 = morris_screening(&circuit, &hw, &space, &eval_config, morris_config)
        .expect("run 1 should succeed");
    let r2 = morris_screening(&circuit, &hw, &space, &eval_config, morris_config)
        .expect("run 2 should succeed");

    assert_eq!(r1.evaluations, r2.evaluations);
    assert_eq!(r1.parameters.len(), r2.parameters.len());

    for (p1, p2) in r1.parameters.iter().zip(r2.parameters.iter()) {
        assert_eq!(p1.name, p2.name);
        assert_eq!(p1.mu, p2.mu, "mu mismatch for {}", p1.name);
        assert_eq!(p1.mu_star, p2.mu_star, "mu_star mismatch for {}", p1.name);
        assert_eq!(p1.sigma, p2.sigma, "sigma mismatch for {}", p1.name);
        assert_eq!(
            p1.elementary_effects, p2.elementary_effects,
            "EE mismatch for {}",
            p1.name
        );
    }
}

#[test]
fn morris_evaluation_count() {
    let circuit = t_chain_circuit();
    let hw = cultivation_hw();
    let space = cultivation_space();
    let eval_config = default_eval_config(7);

    for trajectories in [2, 5, 10] {
        let config = MorrisConfig {
            trajectories,
            levels: 4,
        };
        let result = morris_screening(&circuit, &hw, &space, &eval_config, config)
            .expect("screening should succeed");

        let dim = space.dim() as u64;
        let expected = u64::from(trajectories) * (dim + 1);
        assert_eq!(
            result.evaluations, expected,
            "R={trajectories}: expected {expected} evaluations, got {}",
            result.evaluations
        );
    }
}

#[test]
fn morris_negligible_param() {
    let circuit = clifford_circuit();
    let hw = cultivation_hw();

    let space = ParameterSpace::new(vec![ParameterDef {
        name: "fixup_cost_cycles".into(),
        min: 1.0,
        max: 2.0,
        kind: ParameterKind::Integer,
    }])
    .expect("valid space");

    // StallCount is exactly 0 for Clifford-only circuits regardless of seed,
    // so any parameter variation produces zero elementary effects.
    let eval_config = EvalConfig {
        mc_replicas: 1,
        base_seed: 42,
        max_cycles: Some(50_000),
        metric: OutputMetric::StallCount,
    };
    let config = MorrisConfig {
        trajectories: 4,
        levels: 4,
    };

    let result =
        morris_screening(&circuit, &hw, &space, &eval_config, config).expect("should succeed");

    let fixup = &result.parameters[0];
    assert!(
        fixup.mu_star.abs() < 1e-6,
        "fixup_cost_cycles should have ~0 influence on Clifford-only circuit: mu_star={}",
        fixup.mu_star
    );
}

// ── Property tests ──────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    #[test]
    fn morris_mu_star_ge_abs_mu(seed in any::<u64>()) {
        let circuit = t_chain_circuit();
        let hw = cultivation_hw();
        let space = cultivation_space();
        let eval_config = default_eval_config(seed);
        let config = MorrisConfig {
            trajectories: 3,
            levels: 4,
        };

        let result = morris_screening(&circuit, &hw, &space, &eval_config, config)
            .expect("screening should succeed");

        for p in &result.parameters {
            prop_assert!(
                p.mu_star >= p.mu.abs() - 1e-10,
                "mu_star ({}) must be >= |mu| ({}) for param {} (seed={seed})",
                p.mu_star,
                p.mu.abs(),
                p.name
            );
        }
    }

    #[test]
    fn morris_sigma_nonneg(seed in any::<u64>()) {
        let circuit = t_chain_circuit();
        let hw = cultivation_hw();
        let space = cultivation_space();
        let eval_config = default_eval_config(seed);
        let config = MorrisConfig {
            trajectories: 3,
            levels: 4,
        };

        let result = morris_screening(&circuit, &hw, &space, &eval_config, config)
            .expect("screening should succeed");

        for p in &result.parameters {
            prop_assert!(
                p.sigma >= 0.0,
                "sigma ({}) must be >= 0 for param {} (seed={seed})",
                p.sigma,
                p.name
            );
        }
    }

    #[test]
    fn morris_evaluations_match(
        seed in any::<u64>(),
        trajectories in 2u32..=5,
    ) {
        let circuit = t_chain_circuit();
        let hw = cultivation_hw();
        let space = cultivation_space();
        let eval_config = default_eval_config(seed);
        let config = MorrisConfig {
            trajectories,
            levels: 4,
        };

        let result = morris_screening(&circuit, &hw, &space, &eval_config, config)
            .expect("screening should succeed");

        let dim = space.dim() as u64;
        let expected = u64::from(trajectories) * (dim + 1);
        prop_assert_eq!(
            result.evaluations,
            expected,
            "evaluations should be R * (dim + 1) for seed={}, R={}",
            seed,
            trajectories
        );
    }
}
