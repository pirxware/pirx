//! Sample point evaluation — maps unit-hypercube points to physical
//! parameters, runs the simulation engine, and extracts scalar metrics.

use pirx_core::{Engine, EngineConfig, MonteCarloConfig, run_monte_carlo, trace_summary};
use pirx_hw::model::HardwareModel;
use pirx_ir::ValidatedCircuit;
use serde::{Deserialize, Serialize};

use crate::{
    error::SensitivityError, metric::OutputMetric, mutate::mutate_hw_multi,
    parameter::ParameterSpace,
};

/// Configuration for a single sample-point evaluation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EvalConfig {
    /// Number of Monte Carlo replicas per point. 1 = single run.
    pub mc_replicas: u32,
    /// Base seed for deterministic seed partitioning.
    pub base_seed: u64,
    /// Maximum simulation cycles per replica (`None` = run to completion).
    pub max_cycles: Option<u64>,
    /// Which scalar metric to extract from the simulation.
    pub metric: OutputMetric,
}

/// Evaluate a single point in the unit hypercube.
///
/// Maps `unit_point` through `space` to physical parameters, mutates the
/// hardware model, runs the engine (single or Monte Carlo), and extracts
/// the configured metric.
///
/// Seed partitioning: point N uses seeds
/// `[base + N*mc_replicas, base + (N+1)*mc_replicas)`.
/// Non-overlapping across points. Single-run mode (`mc_replicas <= 1`)
/// is consistent with the first replica of MC mode.
#[allow(clippy::cast_possible_truncation)]
pub fn evaluate_point(
    circuit: &ValidatedCircuit,
    base_hw: &HardwareModel,
    space: &ParameterSpace,
    unit_point: &[f64],
    point_index: usize,
    config: &EvalConfig,
) -> Result<f64, SensitivityError> {
    let physical = space.map_point(unit_point);
    let hw = mutate_hw_multi(base_hw, space, &physical)?;
    let factory_count = hw.factory.count().min(u32::from(u16::MAX)) as u16;

    if config.mc_replicas <= 1 {
        let seed = config
            .base_seed
            .wrapping_add((point_index as u64).wrapping_mul(u64::from(config.mc_replicas.max(1))));
        let engine = Engine::new(
            circuit,
            &hw,
            EngineConfig {
                seed,
                max_cycles: config.max_cycles,
            },
        )?;
        let trace = engine.run();
        let summary = trace_summary(&trace, seed, factory_count);
        return Ok(config.metric.extract(&summary));
    }

    let mc_config = MonteCarloConfig {
        replicas: config.mc_replicas,
        base_seed: config
            .base_seed
            .wrapping_add((point_index as u64).wrapping_mul(u64::from(config.mc_replicas))),
        max_cycles: config.max_cycles,
        threads: None,
    };
    let result = run_monte_carlo(circuit, &hw, mc_config)?;
    #[allow(clippy::cast_precision_loss)]
    let mean = result
        .replicas
        .iter()
        .map(|s| config.metric.extract(s))
        .sum::<f64>()
        / result.replicas.len() as f64;
    Ok(mean)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;
    use crate::parameter::{ParameterDef, ParameterKind};

    fn clifford_eval_setup() -> (ValidatedCircuit, HardwareModel, ParameterSpace, EvalConfig) {
        let circuit = pirx_testkit::validated(pirx_testkit::single_clifford());
        let hw = pirx_testkit::distillation_hw();
        let space = ParameterSpace::new(vec![ParameterDef {
            name: "factory_count".to_owned(),
            min: 1.0,
            max: 4.0,
            kind: ParameterKind::Integer,
        }])
        .unwrap();
        let config = EvalConfig {
            mc_replicas: 1,
            base_seed: 42,
            max_cycles: Some(100_000),
            metric: OutputMetric::TotalCycles,
        };
        (circuit, hw, space, config)
    }

    fn t_chain_eval_setup() -> (ValidatedCircuit, HardwareModel, ParameterSpace, EvalConfig) {
        let circuit = pirx_testkit::validated(pirx_testkit::t_gate_chain(5));
        let hw = pirx_testkit::distillation_hw();
        let space = ParameterSpace::new(vec![ParameterDef {
            name: "factory_count".to_owned(),
            min: 1.0,
            max: 4.0,
            kind: ParameterKind::Integer,
        }])
        .unwrap();
        let config = EvalConfig {
            mc_replicas: 5,
            base_seed: 42,
            max_cycles: Some(100_000),
            metric: OutputMetric::TotalCycles,
        };
        (circuit, hw, space, config)
    }

    #[test]
    fn evaluate_point_single_run() {
        let (circuit, hw, space, config) = clifford_eval_setup();
        let result = evaluate_point(&circuit, &hw, &space, &[0.5], 0, &config).unwrap();
        assert!(result > 0.0, "total_cycles should be > 0, got {result}");
    }

    #[test]
    fn evaluate_point_mc() {
        let (circuit, hw, space, config) = t_chain_eval_setup();
        let result = evaluate_point(&circuit, &hw, &space, &[0.5], 0, &config).unwrap();
        assert!(
            result > 0.0,
            "mean total_cycles should be > 0, got {result}"
        );
    }

    #[test]
    fn evaluate_point_deterministic() {
        let (circuit, hw, space, config) = clifford_eval_setup();
        let r1 = evaluate_point(&circuit, &hw, &space, &[0.5], 0, &config).unwrap();
        let r2 = evaluate_point(&circuit, &hw, &space, &[0.5], 0, &config).unwrap();
        assert!(
            (r1 - r2).abs() < f64::EPSILON,
            "same inputs must produce same output: {r1} != {r2}"
        );
    }

    #[test]
    fn trace_summary_accessible() {
        let circuit = pirx_testkit::validated(pirx_testkit::single_clifford());
        let hw = pirx_testkit::distillation_hw();
        let engine = Engine::new(
            &circuit,
            &hw,
            EngineConfig {
                seed: 42,
                max_cycles: None,
            },
        )
        .unwrap();
        let trace = engine.run();
        let factory_count = hw.factory.count().min(u32::from(u16::MAX)) as u16;
        let summary = trace_summary(&trace, 42, factory_count);
        assert!(summary.total_cycles > 0);
    }
}
