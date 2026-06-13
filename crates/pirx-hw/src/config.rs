//! Hardware model configuration types — the 1:1 mapping to TOML sections.

use serde::Deserialize;

// ── Enums ────────────────────────────────────────────────────────────────────

/// Quantum error-correcting code family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CodeType {
    #[serde(rename = "surface_code")]
    SurfaceCode,
    #[serde(rename = "color_code")]
    ColorCode,
    #[serde(rename = "qldpc")]
    Qldpc,
}

/// Magic state distillation protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum DistillationProtocol {
    #[serde(rename = "15-to-1")]
    FifteenToOne,
    #[serde(rename = "CCZ-to-2T")]
    CczToTwoT,
}

// ── Config types ────────────────────────────────────────────────────────────

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
    pub code_type: CodeType,
    pub code_distance: u32,
    pub physical_error_rate: f64,
    /// Error correction threshold (p_th). Defaults to 0.01 for surface code.
    #[serde(default = "default_error_correction_threshold")]
    pub error_correction_threshold: f64,
    /// Logical error prefactor: p_L = prefactor × (p / p_th)^((d+1)/2).
    #[serde(default = "default_logical_error_prefactor")]
    pub logical_error_prefactor: f64,
}

impl QecConfig {
    /// Logical error probability per magic state.
    ///
    /// p_L = prefactor × (p_phys / p_threshold)^((d+1)/2)
    ///
    /// Returns 0.0 if the result is non-finite or negative.
    pub fn logical_error_rate(&self) -> f64 {
        let ratio = self.physical_error_rate / self.error_correction_threshold;
        let exponent = f64::from(self.code_distance + 1) / 2.0;
        let p_l = self.logical_error_prefactor * ratio.powf(exponent);
        if p_l.is_finite() && p_l >= 0.0 {
            p_l
        } else {
            0.0
        }
    }
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
        protocol: DistillationProtocol,
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

/// Routing model — tagged enum by `model` field.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "model")]
pub enum RoutingConfig {
    /// Fixed overhead per multi-qubit gate. Ignores topology.
    #[serde(rename = "scalar")]
    Scalar {
        #[serde(default = "default_overhead_fraction")]
        overhead_fraction: f64,
    },
    /// Manhattan distance on a logical qubit grid.
    #[serde(rename = "manhattan")]
    Manhattan {
        grid_width: u32,
        grid_height: u32,
        #[serde(default = "default_cycles_per_hop")]
        cycles_per_hop: u32,
    },
}

fn default_overhead_fraction() -> f64 {
    0.5
}

fn default_cycles_per_hop() -> u32 {
    1
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self::Scalar {
            overhead_fraction: 0.5,
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
