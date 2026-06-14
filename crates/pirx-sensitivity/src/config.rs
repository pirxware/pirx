//! TOML configuration parsing for sensitivity analysis.

use serde::{Deserialize, Serialize};

use crate::{error::SensitivityError, metric::OutputMetric, parameter::ParameterDef};

/// Top-level sensitivity analysis configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensitivityConfig {
    pub sweep: SweepConfig,
    pub parameters: Vec<ParameterDef>,
}

/// Sweep-level settings: which metric, how many replicas, which methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepConfig {
    pub metric: OutputMetric,
    pub mc_replicas: u32,
    pub base_seed: u64,
    pub max_cycles: Option<u64>,
    pub morris: Option<MorrisConfig>,
    pub sobol: Option<SobolConfig>,
}

/// Configuration for the Morris elementary effects method.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MorrisConfig {
    /// Number of trajectories (R). Must be >= 2.
    pub trajectories: u32,
    /// Number of levels (p). Must be even and >= 4.
    pub levels: u32,
}

fn default_confidence() -> f64 {
    0.95
}

fn default_bootstrap() -> u32 {
    1000
}

/// Configuration for Sobol variance-based sensitivity indices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SobolConfig {
    /// Base sample size (N). Must be a power of 2.
    pub n_samples: u32,
    /// Confidence level for bootstrap intervals.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Number of bootstrap resamples.
    #[serde(default = "default_bootstrap")]
    pub bootstrap_resamples: u32,
}

/// Parse a TOML string into a [`SensitivityConfig`].
pub fn parse_sensitivity_config(toml_str: &str) -> Result<SensitivityConfig, SensitivityError> {
    toml::from_str(toml_str).map_err(|e| SensitivityError::ConfigParse(e.to_string()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::parameter::ParameterKind;

    const FULL_TOML: &str = r#"
[sweep]
metric = "total_cycles"
mc_replicas = 10
base_seed = 42
max_cycles = 1_000_000

[sweep.morris]
trajectories = 20
levels = 4

[sweep.sobol]
n_samples = 1024
confidence = 0.95
bootstrap_resamples = 1000

[[parameters]]
name = "factory_count"
min = 2
max = 16
kind = "integer"

[[parameters]]
name = "code_distance"
min = 7
max = 25
kind = "odd_integer"

[[parameters]]
name = "overhead_cycles"
min = 1
max = 10
kind = "integer"
"#;

    #[test]
    fn parse_full_config() {
        let config = parse_sensitivity_config(FULL_TOML).unwrap();

        assert_eq!(config.sweep.metric, OutputMetric::TotalCycles);
        assert_eq!(config.sweep.mc_replicas, 10);
        assert_eq!(config.sweep.base_seed, 42);
        assert_eq!(config.sweep.max_cycles, Some(1_000_000));

        let morris = config.sweep.morris.unwrap();
        assert_eq!(morris.trajectories, 20);
        assert_eq!(morris.levels, 4);

        let sobol = config.sweep.sobol.unwrap();
        assert_eq!(sobol.n_samples, 1024);
        assert!((sobol.confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(sobol.bootstrap_resamples, 1000);

        assert_eq!(config.parameters.len(), 3);
        assert_eq!(config.parameters[0].name, "factory_count");
        assert_eq!(config.parameters[0].kind, ParameterKind::Integer);
        assert!((config.parameters[0].min - 2.0).abs() < f64::EPSILON);
        assert!((config.parameters[0].max - 16.0).abs() < f64::EPSILON);

        assert_eq!(config.parameters[1].name, "code_distance");
        assert_eq!(config.parameters[1].kind, ParameterKind::OddInteger);

        assert_eq!(config.parameters[2].name, "overhead_cycles");
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[sweep]
metric = "stall_count"
mc_replicas = 1
base_seed = 0

[[parameters]]
name = "factory_count"
min = 1
max = 8
kind = "integer"
"#;
        let config = parse_sensitivity_config(toml).unwrap();
        assert_eq!(config.sweep.metric, OutputMetric::StallCount);
        assert!(config.sweep.max_cycles.is_none());
        assert!(config.sweep.morris.is_none());
        assert!(config.sweep.sobol.is_none());
        assert_eq!(config.parameters.len(), 1);
    }

    #[test]
    fn parse_morris_only() {
        let toml = r#"
[sweep]
metric = "total_cycles"
mc_replicas = 5
base_seed = 99

[sweep.morris]
trajectories = 10
levels = 6

[[parameters]]
name = "factory_count"
min = 1
max = 8
kind = "integer"
"#;
        let config = parse_sensitivity_config(toml).unwrap();
        assert!(config.sweep.morris.is_some());
        assert!(config.sweep.sobol.is_none());
        assert_eq!(config.sweep.morris.unwrap().levels, 6);
    }

    #[test]
    fn parse_sobol_only() {
        let toml = r#"
[sweep]
metric = "mean_factory_utilization"
mc_replicas = 10
base_seed = 7

[sweep.sobol]
n_samples = 512

[[parameters]]
name = "factory_count"
min = 1
max = 8
kind = "integer"
"#;
        let config = parse_sensitivity_config(toml).unwrap();
        assert!(config.sweep.sobol.is_some());
        assert!(config.sweep.morris.is_none());
        let sobol = config.sweep.sobol.unwrap();
        assert_eq!(sobol.n_samples, 512);
        assert!((sobol.confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(sobol.bootstrap_resamples, 1000);
    }

    #[test]
    fn parse_rejects_unknown_kind() {
        let toml = r#"
[sweep]
metric = "total_cycles"
mc_replicas = 1
base_seed = 0

[[parameters]]
name = "factory_count"
min = 1
max = 8
kind = "banana"
"#;
        let err = parse_sensitivity_config(toml);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("banana") || msg.contains("unknown variant"),
            "error should reference the bad kind: {msg}"
        );
    }

    #[test]
    fn parse_rejects_unknown_metric() {
        let toml = r#"
[sweep]
metric = "banana"
mc_replicas = 1
base_seed = 0

[[parameters]]
name = "factory_count"
min = 1
max = 8
kind = "integer"
"#;
        let err = parse_sensitivity_config(toml);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("banana") || msg.contains("unknown variant"),
            "error should reference the bad metric: {msg}"
        );
    }
}
