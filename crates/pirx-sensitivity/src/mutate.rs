//! Hardware model mutation — apply parameter point to a base model.

use pirx_hw::model::{FactoryConfig, HardwareModel, RoutingConfig};

use crate::{error::SensitivityError, parameter::ParameterSpace};

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn apply_single(
    hw: &mut HardwareModel,
    param_name: &str,
    value: f64,
) -> Result<(), SensitivityError> {
    match param_name {
        "code_distance" => hw.qec.code_distance = value.round() as u32,
        "physical_error_rate" => hw.qec.physical_error_rate = value,
        "error_correction_threshold" => hw.qec.error_correction_threshold = value,
        "logical_error_prefactor" => hw.qec.logical_error_prefactor = value,
        "cycle_time_us" => hw.timing.cycle_time_us = value,
        "measurement_time_us" => hw.timing.measurement_time_us = value,
        "feedback_latency_us" => hw.timing.classical_feedback_latency_us = value,
        "factory_count" => {
            let count = value.round() as u32;
            match &mut hw.factory {
                FactoryConfig::Cultivation { count: c, .. }
                | FactoryConfig::Distillation { count: c, .. }
                | FactoryConfig::RzSynthesis { count: c, .. } => *c = count,
            }
        }
        "lambda_raw" => match &mut hw.factory {
            FactoryConfig::Cultivation { lambda_raw: lr, .. } => *lr = value,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "cultivation",
                });
            }
        },
        "fault_distance" => match &mut hw.factory {
            FactoryConfig::Cultivation {
                fault_distance: fd, ..
            } => *fd = value.round() as u32,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "cultivation",
                });
            }
        },
        "abort_probability" => match &mut hw.factory {
            FactoryConfig::Distillation {
                abort_probability: ap,
                ..
            } => *ap = value,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "distillation",
                });
            }
        },
        "cycles_per_round" => match &mut hw.factory {
            FactoryConfig::Distillation {
                cycles_per_round: cpr,
                ..
            } => *cpr = value.round() as u32,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "distillation",
                });
            }
        },
        "rounds" => match &mut hw.factory {
            FactoryConfig::Distillation { rounds: r, .. } => *r = value.round() as u32,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "distillation",
                });
            }
        },
        "mean_cycles_per_state" => match &mut hw.factory {
            FactoryConfig::RzSynthesis {
                mean_cycles_per_state: mc,
                ..
            } => *mc = value,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "rz_synthesis",
                });
            }
        },
        "distinct_angles" => match &mut hw.factory {
            FactoryConfig::RzSynthesis {
                distinct_angles: da,
                ..
            } => *da = value.round() as u32,
            _ => {
                return Err(SensitivityError::FactoryTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "rz_synthesis",
                });
            }
        },
        "injection_error_probability" => hw.injection.error_probability = value,
        "fixup_cost_cycles" => hw.injection.fixup_cost_cycles = value.round() as u32,
        "overhead_cycles" => match &mut hw.routing {
            RoutingConfig::Scalar {
                overhead_cycles: oc,
            } => *oc = value.round() as u32,
            _ => {
                return Err(SensitivityError::RoutingTypeMismatch {
                    param: param_name.to_owned(),
                    expected: "scalar",
                });
            }
        },
        "buffer_capacity" => hw.buffer.capacity = value.round() as u32,
        "buffer_preload" => hw.buffer.preload = value.round() as u32,
        unknown => return Err(SensitivityError::UnknownParameter(unknown.to_owned())),
    }
    Ok(())
}

/// Clone the base model, apply a single parameter override, and validate.
pub fn mutate_hw(
    base: &HardwareModel,
    param_name: &str,
    value: f64,
) -> Result<HardwareModel, SensitivityError> {
    let mut hw = base.clone();
    apply_single(&mut hw, param_name, value)?;
    hw.validate()
        .map_err(SensitivityError::HardwareValidation)?;
    Ok(hw)
}

/// Clone the base model, apply all parameter overrides from a space, and validate once.
pub fn mutate_hw_multi(
    base: &HardwareModel,
    space: &ParameterSpace,
    values: &[f64],
) -> Result<HardwareModel, SensitivityError> {
    if values.len() != space.dim() {
        return Err(SensitivityError::DimensionMismatch {
            expected: space.dim(),
            actual: values.len(),
        });
    }
    let mut hw = base.clone();
    for (i, param) in space.params().iter().enumerate() {
        #[allow(clippy::indexing_slicing)]
        apply_single(&mut hw, &param.name, values[i])?;
    }
    hw.validate()
        .map_err(SensitivityError::HardwareValidation)?;
    Ok(hw)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::parameter::KNOWN_PARAMS;

    #[test]
    fn apply_single_handles_all_known_params() {
        let hw = pirx_testkit::cultivation_hw();
        for &name in KNOWN_PARAMS {
            let mut hw_copy = hw.clone();
            let result = apply_single(&mut hw_copy, name, 1.0);
            assert!(
                !matches!(result, Err(SensitivityError::UnknownParameter(_))),
                "KNOWN_PARAMS entry '{name}' not handled by apply_single — \
                 got UnknownParameter instead of Ok or type mismatch"
            );
        }
    }
}
