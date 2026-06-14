//! Evaluation overhead sanity check.
//!
//! Runs evaluate_point repeatedly on a small circuit to verify that
//! parameter mapping + HW mutation + validation overhead is negligible
//! compared to engine runtime.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use std::time::Instant;

use pirx_core::{Engine, EngineConfig};
use pirx_sensitivity::{
    EvalConfig, OutputMetric, ParameterDef, ParameterKind, ParameterSpace, evaluate_point,
};

#[test]
#[ignore]
fn evaluate_point_overhead_under_one_percent() {
    let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(500));
    let hw = pirx_testkit::distillation_hw();

    let space = ParameterSpace::new(vec![
        ParameterDef {
            name: "factory_count".into(),
            min: 1.0,
            max: 4.0,
            kind: ParameterKind::Integer,
        },
        ParameterDef {
            name: "buffer_capacity".into(),
            min: 2.0,
            max: 8.0,
            kind: ParameterKind::Integer,
        },
    ])
    .expect("valid space");

    let eval_config = EvalConfig {
        mc_replicas: 1,
        base_seed: 42,
        max_cycles: Some(100_000),
        metric: OutputMetric::TotalCycles,
    };

    const N: usize = 100;

    // Baseline: raw engine runs (no parameter mapping, no mutation)
    let baseline_start = Instant::now();
    for i in 0..N {
        let engine = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed: 42u64.wrapping_add(i as u64),
                max_cycles: Some(100_000),
            },
        )
        .expect("engine construction");
        let _ = engine.run();
    }
    let baseline_elapsed = baseline_start.elapsed();

    // With overhead: evaluate_point (mapping + mutation + validation + engine)
    let eval_start = Instant::now();
    for i in 0..N {
        let _ = evaluate_point(&circuit, &hw, &space, &[0.5, 0.5], i, &eval_config)
            .expect("evaluate_point");
    }
    let eval_elapsed = eval_start.elapsed();

    let overhead_ns = eval_elapsed
        .as_nanos()
        .saturating_sub(baseline_elapsed.as_nanos());
    let overhead_pct = (overhead_ns as f64 / eval_elapsed.as_nanos() as f64) * 100.0;

    eprintln!(
        "baseline: {baseline_elapsed:?}, with eval: {eval_elapsed:?}, overhead: {overhead_pct:.2}%"
    );
    assert!(
        overhead_pct < 1.0,
        "evaluation overhead {overhead_pct:.2}% exceeds 1% threshold"
    );
}
