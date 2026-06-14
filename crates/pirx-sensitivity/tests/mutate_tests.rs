//! Mutation tests — applying parameter overrides to hardware models.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use pirx_hw::model::RoutingConfig;
use pirx_sensitivity::{
    ParameterDef, ParameterKind, ParameterSpace, SensitivityError, mutate_hw, mutate_hw_multi,
};
use pirx_testkit::{cultivation_hw, distillation_hw, manhattan_hw};

#[test]
fn mutate_factory_count() {
    let base = cultivation_hw();
    let hw = mutate_hw(&base, "factory_count", 8.0).expect("valid mutation");
    assert_eq!(hw.factory.count(), 8);
}

#[test]
fn mutate_code_distance() {
    let base = cultivation_hw();
    let hw = mutate_hw(&base, "code_distance", 9.0).expect("valid mutation");
    assert_eq!(hw.qec.code_distance, 9);
}

#[test]
fn mutate_code_distance_even_rejected() {
    let base = cultivation_hw();
    let result = mutate_hw(&base, "code_distance", 8.0);
    assert!(
        matches!(result, Err(SensitivityError::HardwareValidation(_))),
        "even code distance should fail HW validation: {result:?}"
    );
}

#[test]
fn mutate_buffer_capacity() {
    let base = cultivation_hw();
    let hw = mutate_hw(&base, "buffer_capacity", 16.0).expect("valid mutation");
    assert_eq!(hw.buffer.capacity, 16);
}

#[test]
fn mutate_overhead_cycles() {
    let base = cultivation_hw();
    let hw = mutate_hw(&base, "overhead_cycles", 3.0).expect("valid mutation");
    match hw.routing {
        RoutingConfig::Scalar { overhead_cycles } => assert_eq!(overhead_cycles, 3),
        _ => panic!("expected Scalar routing"),
    }
}

#[test]
fn mutate_continuous_param() {
    let base = cultivation_hw();
    let hw = mutate_hw(&base, "injection_error_probability", 0.3).expect("valid mutation");
    assert!((hw.injection.error_probability - 0.3).abs() < f64::EPSILON);
}

#[test]
fn mutate_multi_params() {
    let base = cultivation_hw();
    let space = ParameterSpace::new(vec![
        ParameterDef {
            name: "factory_count".into(),
            min: 1.0,
            max: 8.0,
            kind: ParameterKind::Integer,
        },
        ParameterDef {
            name: "buffer_capacity".into(),
            min: 2.0,
            max: 16.0,
            kind: ParameterKind::Integer,
        },
    ])
    .expect("valid space");

    let hw = mutate_hw_multi(&base, &space, &[4.0, 12.0]).expect("valid mutation");
    assert_eq!(hw.factory.count(), 4);
    assert_eq!(hw.buffer.capacity, 12);
}

#[test]
fn mutate_unknown_rejected() {
    let base = cultivation_hw();
    let result = mutate_hw(&base, "banana", 42.0);
    assert!(
        matches!(result, Err(SensitivityError::UnknownParameter(ref name)) if name == "banana"),
        "unknown parameter should be rejected: {result:?}"
    );
}

#[test]
fn mutate_factory_type_mismatch() {
    let base = distillation_hw();
    let result = mutate_hw(&base, "lambda_raw", 0.005);
    assert!(
        matches!(
            result,
            Err(SensitivityError::FactoryTypeMismatch { expected, .. }) if expected == "cultivation"
        ),
        "lambda_raw on distillation should fail: {result:?}"
    );
}

#[test]
fn mutate_routing_type_mismatch() {
    let base = manhattan_hw(4, 4);
    let result = mutate_hw(&base, "overhead_cycles", 3.0);
    assert!(
        matches!(
            result,
            Err(SensitivityError::RoutingTypeMismatch { expected, .. }) if expected == "scalar"
        ),
        "overhead_cycles on manhattan should fail: {result:?}"
    );
}

#[test]
fn dimension_mismatch() {
    let base = cultivation_hw();
    let space = ParameterSpace::new(vec![
        ParameterDef {
            name: "factory_count".into(),
            min: 1.0,
            max: 8.0,
            kind: ParameterKind::Integer,
        },
        ParameterDef {
            name: "buffer_capacity".into(),
            min: 2.0,
            max: 16.0,
            kind: ParameterKind::Integer,
        },
    ])
    .expect("valid space");

    let result = mutate_hw_multi(&base, &space, &[4.0]);
    assert!(
        matches!(
            result,
            Err(SensitivityError::DimensionMismatch {
                expected: 2,
                actual: 1
            })
        ),
        "wrong number of values should fail: {result:?}"
    );
}

#[test]
fn roundtrip_unit_to_physical() {
    let base = cultivation_hw();

    // Continuous: injection_error_probability [0.1, 0.9]
    let space_cont = ParameterSpace::new(vec![ParameterDef {
        name: "injection_error_probability".into(),
        min: 0.1,
        max: 0.9,
        kind: ParameterKind::Continuous,
    }])
    .expect("valid space");
    let min_val = space_cont.map_unit_to_physical(0, 0.0);
    let max_val = space_cont.map_unit_to_physical(0, 1.0);
    let hw_min = mutate_hw(&base, "injection_error_probability", min_val).expect("valid");
    let hw_max = mutate_hw(&base, "injection_error_probability", max_val).expect("valid");
    assert!((hw_min.injection.error_probability - 0.1).abs() < f64::EPSILON);
    assert!((hw_max.injection.error_probability - 0.9).abs() < f64::EPSILON);

    // Integer: buffer_capacity [2, 16]
    let space_int = ParameterSpace::new(vec![ParameterDef {
        name: "buffer_capacity".into(),
        min: 2.0,
        max: 16.0,
        kind: ParameterKind::Integer,
    }])
    .expect("valid space");
    let min_val = space_int.map_unit_to_physical(0, 0.0);
    let max_val = space_int.map_unit_to_physical(0, 1.0);
    let hw_min = mutate_hw(&base, "buffer_capacity", min_val).expect("valid");
    let hw_max = mutate_hw(&base, "buffer_capacity", max_val).expect("valid");
    assert_eq!(hw_min.buffer.capacity, 2);
    assert_eq!(hw_max.buffer.capacity, 16);

    // OddInteger: code_distance [3, 21]
    let space_odd = ParameterSpace::new(vec![ParameterDef {
        name: "code_distance".into(),
        min: 3.0,
        max: 21.0,
        kind: ParameterKind::OddInteger,
    }])
    .expect("valid space");
    let min_val = space_odd.map_unit_to_physical(0, 0.0);
    let max_val = space_odd.map_unit_to_physical(0, 1.0);
    let hw_min = mutate_hw(&base, "code_distance", min_val).expect("valid");
    let hw_max = mutate_hw(&base, "code_distance", max_val).expect("valid");
    assert_eq!(hw_min.qec.code_distance, 3);
    assert_eq!(hw_max.qec.code_distance, 21);
}
