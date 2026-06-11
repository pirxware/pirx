//! Hardware model types, deserialized from TOML.

use serde::Deserialize;

/// Complete hardware model specification.
#[derive(Debug, Clone, Deserialize)]
pub struct HardwareModel {
    pub meta: MetaConfig,
    pub qec: QecConfig,
    pub timing: TimingConfig,
    pub factory: FactoryConfig,
    pub injection: InjectionConfig,
    pub routing: RoutingConfig,
    pub buffer: BufferConfig,
}

/// Model metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Quantum error correction parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct QecConfig {
    pub code_type: String,
    pub code_distance: u32,
    pub physical_error_rate: f64,
    /// Error correction threshold (p_th). Defaults to 0.01 for surface code.
    #[serde(default = "default_error_correction_threshold")]
    pub error_correction_threshold: f64,
    /// Logical error prefactor: p_L = prefactor × (p / p_th)^((d+1)/2).
    #[serde(default = "default_logical_error_prefactor")]
    pub logical_error_prefactor: f64,
}

fn default_error_correction_threshold() -> f64 {
    0.01
}

fn default_logical_error_prefactor() -> f64 {
    0.038
}

/// Timing parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TimingConfig {
    /// One QEC round in microseconds.
    pub cycle_time_us: f64,
    /// Measurement duration in microseconds.
    #[serde(default = "default_measurement_time")]
    pub measurement_time_us: f64,
    /// Decoder + classical processing latency in microseconds.
    #[serde(default = "default_feedback_latency")]
    pub classical_feedback_latency_us: f64,
}

fn default_measurement_time() -> f64 {
    0.5
}

fn default_feedback_latency() -> f64 {
    1.0
}

/// Factory configuration — tagged enum by `type` field.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum FactoryConfig {
    #[serde(rename = "distillation")]
    Distillation {
        count: u32,
        protocol: String,
        cycles_per_round: u32,
        rounds: u32,
        abort_probability: f64,
    },
    #[serde(rename = "cultivation")]
    Cultivation {
        count: u32,
        lambda_raw: f64,
        fault_distance: u32,
    },
    #[serde(rename = "rz_synthesis")]
    RzSynthesis {
        count: u32,
        distinct_angles: u32,
        mean_cycles_per_state: f64,
    },
}

impl FactoryConfig {
    /// Number of factory instances.
    pub fn count(&self) -> u32 {
        match self {
            Self::Distillation { count, .. }
            | Self::Cultivation { count, .. }
            | Self::RzSynthesis { count, .. } => *count,
        }
    }
}

/// Injection error parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct InjectionConfig {
    #[serde(default = "default_injection_probability")]
    pub error_probability: f64,
    #[serde(default = "default_fixup_cost")]
    pub fixup_cost_cycles: u32,
}

fn default_injection_probability() -> f64 {
    0.5
}
fn default_fixup_cost() -> u32 {
    1
}

/// Routing model parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    /// Routing model type: "scalar" or "graph".
    #[serde(default = "default_routing_model")]
    pub model: String,
    /// For scalar model: fraction of extra qubits used as routing overhead.
    #[serde(default = "default_overhead_fraction")]
    pub overhead_fraction: f64,
}

fn default_routing_model() -> String {
    "scalar".to_owned()
}

fn default_overhead_fraction() -> f64 {
    0.5
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            model: default_routing_model(),
            overhead_fraction: default_overhead_fraction(),
        }
    }
}

/// Magic state buffer parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct BufferConfig {
    pub capacity: u32,
    #[serde(default)]
    pub preload: u32,
}

/// Load a hardware model from a TOML string.
///
/// # Errors
///
/// Returns an error if the TOML is malformed or missing required fields.
pub fn load(toml_str: &str) -> Result<HardwareModel, toml::de::Error> {
    toml::from_str(toml_str)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn parse_cultivation_model() {
        let toml = include_str!("../../../models/surface_code_d17_cultivation.toml");
        let hw = load(toml).unwrap();
        assert_eq!(hw.qec.code_distance, 17);
        assert!(matches!(hw.factory, FactoryConfig::Cultivation { .. }));
    }

    #[test]
    fn parse_distillation_model() {
        let toml = include_str!("../../../models/surface_code_d17_distillation.toml");
        let hw = load(toml).unwrap();
        assert!(matches!(hw.factory, FactoryConfig::Distillation { .. }));
    }

    #[test]
    fn defaults_applied_for_optional_fields() {
        let toml = r#"
[meta]
name = "minimal"

[qec]
code_type = "surface_code"
code_distance = 9
physical_error_rate = 1e-3

[timing]
cycle_time_us = 1.0

[factory]
type = "cultivation"
count = 4
lambda_raw = 0.002
fault_distance = 3

[injection]

[routing]

[buffer]
capacity = 4
"#;
        let hw = load(toml).unwrap();
        assert!((hw.injection.error_probability - 0.5).abs() < f64::EPSILON);
        assert_eq!(hw.injection.fixup_cost_cycles, 1);
        assert_eq!(hw.routing.model, "scalar");
        assert!((hw.routing.overhead_fraction - 0.5).abs() < f64::EPSILON);
        assert!((hw.qec.error_correction_threshold - 0.01).abs() < f64::EPSILON);
    }
}
