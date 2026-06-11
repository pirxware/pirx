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
}

/// Timing parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TimingConfig {
    pub cycle_time_us: f64,
    #[serde(default = "default_feedback_latency")]
    pub classical_feedback_latency_us: f64,
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
