//! Parameter space definition and unit-to-physical mapping.

use std::collections::HashSet;

use pirx_hw::model::{FactoryConfig, HardwareModel, RoutingConfig};
use serde::{Deserialize, Serialize};

use crate::error::SensitivityError;

/// All sweepable hardware model fields.
pub(crate) const KNOWN_PARAMS: &[&str] = &[
    "code_distance",
    "physical_error_rate",
    "error_correction_threshold",
    "logical_error_prefactor",
    "cycle_time_us",
    "measurement_time_us",
    "feedback_latency_us",
    "factory_count",
    "lambda_raw",
    "fault_distance",
    "cycles_per_round",
    "rounds",
    "abort_probability",
    "mean_cycles_per_state",
    "distinct_angles",
    "injection_error_probability",
    "fixup_cost_cycles",
    "overhead_cycles",
    "buffer_capacity",
    "buffer_preload",
];

const CULTIVATION_PARAMS: &[&str] = &["lambda_raw", "fault_distance"];
const DISTILLATION_PARAMS: &[&str] = &["cycles_per_round", "rounds", "abort_probability"];
const RZ_SYNTHESIS_PARAMS: &[&str] = &["mean_cycles_per_state", "distinct_angles"];
const SCALAR_ROUTING_PARAMS: &[&str] = &["overhead_cycles"];

/// How a unit-interval value maps to the physical parameter range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    Continuous,
    Integer,
    OddInteger,
}

fn default_continuous() -> ParameterKind {
    ParameterKind::Continuous
}

/// Definition of a single sweepable parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDef {
    pub name: String,
    pub min: f64,
    pub max: f64,
    #[serde(default = "default_continuous")]
    pub kind: ParameterKind,
}

/// A validated collection of parameter definitions for sensitivity sweeps.
///
/// All parameters have valid ranges and no duplicates. The `map_unit_to_physical`
/// method converts unit-interval samples ([0, 1]) to physical values, respecting
/// the parameter kind (continuous, integer, odd-integer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSpace {
    params: Vec<ParameterDef>,
}

impl ParameterSpace {
    /// Construct a validated parameter space.
    ///
    /// Checks: non-empty, no duplicates, min < max, min >= 0, kind-specific bounds.
    pub fn new(params: Vec<ParameterDef>) -> Result<Self, SensitivityError> {
        let space = Self { params };
        space.validate_internal()?;
        Ok(space)
    }

    /// Number of dimensions (parameters) in this space.
    pub fn dim(&self) -> usize {
        self.params.len()
    }

    /// Borrow the parameter definitions.
    pub fn params(&self) -> &[ParameterDef] {
        &self.params
    }

    /// Map a single unit-interval value to the physical value for parameter `index`.
    ///
    /// `u` is clamped to [0, 1]. The mapping depends on [`ParameterKind`]:
    /// - `Continuous`: linear interpolation `min + u * (max - min)`
    /// - `Integer`: `round(min + u * (max - min))`
    /// - `OddInteger`: uniform discrete mapping to odd integers in [min, max]
    #[allow(clippy::indexing_slicing)]
    pub fn map_unit_to_physical(&self, index: usize, u: f64) -> f64 {
        let p = &self.params[index];
        let u = u.clamp(0.0, 1.0);
        match p.kind {
            ParameterKind::Continuous => p.min + u * (p.max - p.min),
            ParameterKind::Integer => (p.min + u * (p.max - p.min)).round(),
            ParameterKind::OddInteger => {
                // Safety: validate_internal guarantees bounds are odd integers >= 3
                // and min < max, so these casts and arithmetic are well-defined.
                #[allow(clippy::cast_possible_truncation)]
                let min_odd = p.min as i64;
                #[allow(clippy::cast_possible_truncation)]
                let max_odd = p.max as i64;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let n_odds = ((max_odd - min_odd) / 2 + 1) as usize;
                if n_odds <= 1 {
                    return p.min;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let idx = (u * (n_odds - 1) as f64).round() as usize;
                (min_odd + (idx as i64) * 2) as f64
            }
        }
    }

    /// Map a full unit-interval point to physical values.
    pub fn map_point(&self, unit_point: &[f64]) -> Result<Vec<f64>, SensitivityError> {
        if unit_point.len() != self.dim() {
            return Err(SensitivityError::DimensionMismatch {
                expected: self.dim(),
                actual: unit_point.len(),
            });
        }
        Ok((0..self.dim())
            .map(|i| {
                #[allow(clippy::indexing_slicing)]
                self.map_unit_to_physical(i, unit_point[i])
            })
            .collect())
    }

    /// Full validation: internal consistency + compatibility with hardware model.
    pub fn validate(&self, hw: &HardwareModel) -> Result<(), SensitivityError> {
        self.validate_internal()?;
        self.validate_against_hw(hw)?;
        Ok(())
    }

    fn validate_internal(&self) -> Result<(), SensitivityError> {
        if self.params.is_empty() {
            return Err(SensitivityError::EmptyParameterSpace);
        }

        let mut seen = HashSet::new();
        for p in &self.params {
            if !seen.insert(&p.name) {
                return Err(SensitivityError::DuplicateParameter(p.name.clone()));
            }

            if p.min >= p.max {
                return Err(SensitivityError::InvalidRange {
                    name: p.name.clone(),
                    min: p.min,
                    max: p.max,
                });
            }

            if p.min < 0.0 {
                return Err(SensitivityError::NegativeBound {
                    param: p.name.clone(),
                    min: p.min,
                });
            }

            match p.kind {
                ParameterKind::Continuous => {}
                ParameterKind::Integer => {
                    if p.min.fract().abs() > f64::EPSILON || p.max.fract().abs() > f64::EPSILON {
                        return Err(SensitivityError::NonIntegerBound {
                            param: p.name.clone(),
                            min: p.min,
                            max: p.max,
                        });
                    }
                }
                ParameterKind::OddInteger => {
                    #[allow(clippy::cast_possible_truncation)]
                    let min_i = p.min as i64;
                    #[allow(clippy::cast_possible_truncation)]
                    let max_i = p.max as i64;
                    if min_i < 3
                        || min_i % 2 == 0
                        || max_i % 2 == 0
                        || p.min.fract().abs() > f64::EPSILON
                        || p.max.fract().abs() > f64::EPSILON
                    {
                        return Err(SensitivityError::NonOddBound {
                            param: p.name.clone(),
                            min: p.min,
                            max: p.max,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_against_hw(&self, hw: &HardwareModel) -> Result<(), SensitivityError> {
        for p in &self.params {
            if !KNOWN_PARAMS.contains(&p.name.as_str()) {
                return Err(SensitivityError::UnknownParameter(p.name.clone()));
            }

            if CULTIVATION_PARAMS.contains(&p.name.as_str())
                && !matches!(hw.factory, FactoryConfig::Cultivation { .. })
            {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: p.name.clone(),
                    expected: "cultivation",
                });
            }

            if DISTILLATION_PARAMS.contains(&p.name.as_str())
                && !matches!(hw.factory, FactoryConfig::Distillation { .. })
            {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: p.name.clone(),
                    expected: "distillation",
                });
            }

            if RZ_SYNTHESIS_PARAMS.contains(&p.name.as_str())
                && !matches!(hw.factory, FactoryConfig::RzSynthesis { .. })
            {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: p.name.clone(),
                    expected: "rz_synthesis",
                });
            }

            if SCALAR_ROUTING_PARAMS.contains(&p.name.as_str())
                && !matches!(hw.routing, RoutingConfig::Scalar { .. })
            {
                return Err(SensitivityError::RoutingTypeMismatch {
                    param: p.name.clone(),
                    expected: "scalar",
                });
            }
        }

        Ok(())
    }
}
