//! Hardware model types, deserialized from TOML.

use serde::Deserialize;
use thiserror::Error;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors from hardware model loading or validation.
#[derive(Debug, Error)]
pub enum HardwareModelError {
    #[error("TOML parsing failed: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("code distance must be odd and >= 3, got {0}")]
    InvalidCodeDistance(u32),

    #[error("physical error rate must be in (0, 1), got {0}")]
    InvalidPhysicalErrorRate(f64),

    #[error("error correction threshold must be in (0, 1), got {0}")]
    InvalidErrorCorrectionThreshold(f64),

    #[error("logical error prefactor must be > 0, got {0}")]
    InvalidLogicalErrorPrefactor(f64),

    #[error("cycle time must be > 0, got {0} µs")]
    InvalidCycleTime(f64),

    #[error("measurement time must be > 0, got {0} µs")]
    InvalidMeasurementTime(f64),

    #[error("classical feedback latency must be >= 0, got {0} µs")]
    InvalidFeedbackLatency(f64),

    #[error("factory count must be > 0")]
    ZeroFactories,

    #[error("buffer capacity must be > 0")]
    ZeroBufferCapacity,

    #[error("lambda_raw must be > 0 for cultivation factory, got {0}")]
    InvalidLambdaRaw(f64),

    #[error("abort probability must be in [0, 1], got {0}")]
    InvalidAbortProbability(f64),

    #[error("injection error probability must be in [0, 1], got {0}")]
    InvalidInjectionProbability(f64),

    #[error("routing overhead fraction must be in [0, 1], got {0}")]
    InvalidOverheadFraction(f64),
}

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

// ── Model types ──────────────────────────────────────────────────────────────

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

impl HardwareModel {
    /// Validate all domain invariants.
    ///
    /// Called automatically by [`load`]. Callers constructing a `HardwareModel`
    /// directly (e.g. in tests) may call this explicitly for defence-in-depth.
    pub fn validate(&self) -> Result<(), HardwareModelError> {
        // QEC
        if self.qec.code_distance < 3 || self.qec.code_distance.is_multiple_of(2) {
            return Err(HardwareModelError::InvalidCodeDistance(
                self.qec.code_distance,
            ));
        }
        let p = self.qec.physical_error_rate;
        if p <= 0.0 || p >= 1.0 || p.is_nan() {
            return Err(HardwareModelError::InvalidPhysicalErrorRate(p));
        }
        let t = self.qec.error_correction_threshold;
        if t <= 0.0 || t >= 1.0 || t.is_nan() {
            return Err(HardwareModelError::InvalidErrorCorrectionThreshold(t));
        }
        let pf = self.qec.logical_error_prefactor;
        if pf <= 0.0 || pf.is_nan() {
            return Err(HardwareModelError::InvalidLogicalErrorPrefactor(pf));
        }

        // Timing
        let ct = self.timing.cycle_time_us;
        if ct <= 0.0 || ct.is_nan() {
            return Err(HardwareModelError::InvalidCycleTime(ct));
        }
        let mt = self.timing.measurement_time_us;
        if mt <= 0.0 || mt.is_nan() {
            return Err(HardwareModelError::InvalidMeasurementTime(mt));
        }
        let fl = self.timing.classical_feedback_latency_us;
        if fl < 0.0 || fl.is_nan() {
            return Err(HardwareModelError::InvalidFeedbackLatency(fl));
        }

        // Factory
        if self.factory.count() == 0 {
            return Err(HardwareModelError::ZeroFactories);
        }
        match &self.factory {
            FactoryConfig::Cultivation { lambda_raw, .. } => {
                if *lambda_raw <= 0.0 || lambda_raw.is_nan() {
                    return Err(HardwareModelError::InvalidLambdaRaw(*lambda_raw));
                }
            }
            FactoryConfig::Distillation {
                abort_probability, ..
            } => {
                let ap = *abort_probability;
                if !(0.0..=1.0).contains(&ap) || ap.is_nan() {
                    return Err(HardwareModelError::InvalidAbortProbability(ap));
                }
            }
            FactoryConfig::RzSynthesis { .. } => {}
        }

        // Injection
        let ep = self.injection.error_probability;
        if !(0.0..=1.0).contains(&ep) || ep.is_nan() {
            return Err(HardwareModelError::InvalidInjectionProbability(ep));
        }

        // Routing
        match &self.routing {
            RoutingConfig::Scalar { overhead_fraction } => {
                let of = *overhead_fraction;
                if !(0.0..=1.0).contains(&of) || of.is_nan() {
                    return Err(HardwareModelError::InvalidOverheadFraction(of));
                }
            }
            RoutingConfig::Manhattan { .. } => {
                // grid_width, grid_height, cycles_per_hop are u32 — no invalid values.
            }
        }

        // Buffer
        if self.buffer.capacity == 0 {
            return Err(HardwareModelError::ZeroBufferCapacity);
        }

        Ok(())
    }
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
///
/// Mirrors [`RoutingConfig`]: closed set, exhaustive matching, serde-driven.
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

/// Load and validate a hardware model from a TOML string.
///
/// # Errors
///
/// Returns an error if the TOML is malformed, missing required fields,
/// or violates domain invariants (e.g. even code distance, out-of-range
/// probabilities, non-positive rates).
pub fn load(toml_str: &str) -> Result<HardwareModel, HardwareModelError> {
    let hw: HardwareModel = toml::from_str(toml_str)?;
    hw.validate()?;
    Ok(hw)
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
        assert_eq!(hw.qec.code_type, CodeType::SurfaceCode);
        assert!(matches!(hw.factory, FactoryConfig::Cultivation { .. }));
    }

    #[test]
    fn parse_distillation_model() {
        let toml = include_str!("../../../models/surface_code_d17_distillation.toml");
        let hw = load(toml).unwrap();
        assert!(matches!(
            hw.factory,
            FactoryConfig::Distillation {
                protocol: DistillationProtocol::FifteenToOne,
                ..
            }
        ));
    }

    #[test]
    fn parse_manhattan_model() {
        let toml = include_str!("../../../models/surface_code_d17_cultivation_manhattan.toml");
        let hw = load(toml).unwrap();
        match hw.routing {
            RoutingConfig::Manhattan {
                grid_width,
                grid_height,
                cycles_per_hop,
            } => {
                assert_eq!(grid_width, 10);
                assert_eq!(grid_height, 10);
                assert_eq!(cycles_per_hop, 1);
            }
            _ => panic!("expected Manhattan routing"),
        }
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
model = "scalar"

[buffer]
capacity = 4
"#;
        let hw = load(toml).unwrap();
        assert!((hw.injection.error_probability - 0.5).abs() < f64::EPSILON);
        assert_eq!(hw.injection.fixup_cost_cycles, 1);
        match hw.routing {
            RoutingConfig::Scalar { overhead_fraction } => {
                assert!(
                    (overhead_fraction - 0.5).abs() < f64::EPSILON,
                    "scalar overhead_fraction should default to 0.5"
                );
            }
            _ => panic!("expected Scalar routing"),
        }
        assert!((hw.qec.error_correction_threshold - 0.01).abs() < f64::EPSILON);
    }

    // ── Validation tests ─────────────────────────────────────────────────────

    /// Helper: valid TOML that passes all checks. Tests below mutate one field.
    fn valid_toml(factory_section: &str) -> String {
        format!(
            r#"
[meta]
name = "valid"

[qec]
code_type = "surface_code"
code_distance = 9
physical_error_rate = 1e-3

[timing]
cycle_time_us = 1.0

{factory_section}

[injection]
error_probability = 0.5
fixup_cost_cycles = 1

[routing]
model = "scalar"
overhead_fraction = 0.5

[buffer]
capacity = 4
"#
        )
    }

    fn cultivation_factory() -> &'static str {
        "[factory]\ntype = \"cultivation\"\ncount = 4\nlambda_raw = 0.002\nfault_distance = 3"
    }

    #[test]
    fn rejects_even_code_distance() {
        let toml =
            valid_toml(cultivation_factory()).replace("code_distance = 9", "code_distance = 8");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidCodeDistance(8))
        ));
    }

    #[test]
    fn rejects_code_distance_below_three() {
        let toml =
            valid_toml(cultivation_factory()).replace("code_distance = 9", "code_distance = 1");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidCodeDistance(1))
        ));
    }

    #[test]
    fn rejects_zero_physical_error_rate() {
        let toml = valid_toml(cultivation_factory())
            .replace("physical_error_rate = 1e-3", "physical_error_rate = 0.0");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidPhysicalErrorRate(_))
        ));
    }

    #[test]
    fn rejects_negative_cycle_time() {
        let toml = valid_toml(cultivation_factory())
            .replace("cycle_time_us = 1.0", "cycle_time_us = -1.0");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidCycleTime(_))
        ));
    }

    #[test]
    fn rejects_zero_factory_count() {
        let toml = valid_toml(cultivation_factory()).replace("count = 4", "count = 0");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::ZeroFactories)
        ));
    }

    #[test]
    fn rejects_zero_buffer_capacity() {
        let toml = valid_toml(cultivation_factory()).replace("capacity = 4", "capacity = 0");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::ZeroBufferCapacity)
        ));
    }

    #[test]
    fn rejects_negative_lambda_raw() {
        let toml =
            valid_toml(cultivation_factory()).replace("lambda_raw = 0.002", "lambda_raw = -0.1");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidLambdaRaw(_))
        ));
    }

    #[test]
    fn rejects_injection_probability_above_one() {
        let toml = valid_toml(cultivation_factory())
            .replace("error_probability = 0.5", "error_probability = 1.5");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidInjectionProbability(_))
        ));
    }

    #[test]
    fn rejects_overhead_fraction_above_one() {
        let toml = valid_toml(cultivation_factory())
            .replace("overhead_fraction = 0.5", "overhead_fraction = 2.0");
        assert!(matches!(
            load(&toml),
            Err(HardwareModelError::InvalidOverheadFraction(_))
        ));
    }

    #[test]
    fn rejects_invalid_code_type() {
        let toml = valid_toml(cultivation_factory())
            .replace("code_type = \"surface_code\"", "code_type = \"banana\"");
        assert!(matches!(load(&toml), Err(HardwareModelError::Parse(_))));
    }

    #[test]
    fn rejects_invalid_routing_model() {
        let toml =
            valid_toml(cultivation_factory()).replace("model = \"scalar\"", "model = \"magic\"");
        assert!(matches!(load(&toml), Err(HardwareModelError::Parse(_))));
    }
}
