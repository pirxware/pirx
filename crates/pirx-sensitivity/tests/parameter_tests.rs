//! Tests for parameter space definition and unit-to-physical mapping.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use pirx_sensitivity::{ParameterDef, ParameterKind, ParameterSpace, SensitivityError};
use pirx_testkit::cultivation_hw;

#[test]
fn parse_parameter_kind_continuous() {
    let json = r#""continuous""#;
    let kind: ParameterKind = serde_json::from_str(json).expect("deser continuous");
    assert_eq!(kind, ParameterKind::Continuous);
}

#[test]
fn parse_parameter_kind_integer() {
    let json = r#""integer""#;
    let kind: ParameterKind = serde_json::from_str(json).expect("deser integer");
    assert_eq!(kind, ParameterKind::Integer);
}

#[test]
fn parse_parameter_kind_odd_integer() {
    let json = r#""odd_integer""#;
    let kind: ParameterKind = serde_json::from_str(json).expect("deser odd_integer");
    assert_eq!(kind, ParameterKind::OddInteger);
}

#[test]
fn map_unit_continuous() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "physical_error_rate".into(),
        min: 1e-4,
        max: 1e-2,
        kind: ParameterKind::Continuous,
    }])
    .expect("valid space");

    let min_val = space.map_unit_to_physical(0, 0.0);
    let max_val = space.map_unit_to_physical(0, 1.0);
    let mid_val = space.map_unit_to_physical(0, 0.5);

    assert!((min_val - 1e-4).abs() < 1e-15);
    assert!((max_val - 1e-2).abs() < 1e-15);
    assert!((mid_val - 0.005_05).abs() < 1e-15);
}

#[test]
fn map_unit_integer() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "buffer_capacity".into(),
        min: 2.0,
        max: 10.0,
        kind: ParameterKind::Integer,
    }])
    .expect("valid space");

    assert!((space.map_unit_to_physical(0, 0.0) - 2.0).abs() < f64::EPSILON);
    assert!((space.map_unit_to_physical(0, 1.0) - 10.0).abs() < f64::EPSILON);
    assert!((space.map_unit_to_physical(0, 0.5) - 6.0).abs() < f64::EPSILON);
    assert!((space.map_unit_to_physical(0, 0.3) - 4.0).abs() < f64::EPSILON);
}

#[test]
fn map_unit_odd_integer_endpoints() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "code_distance".into(),
        min: 3.0,
        max: 21.0,
        kind: ParameterKind::OddInteger,
    }])
    .expect("valid space");

    assert!((space.map_unit_to_physical(0, 0.0) - 3.0).abs() < f64::EPSILON);
    assert!((space.map_unit_to_physical(0, 1.0) - 21.0).abs() < f64::EPSILON);
}

#[test]
fn map_unit_odd_integer_uniform() {
    // min=7, max=25 → odds are {7,9,11,13,15,17,19,21,23,25} → 10 values
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "code_distance".into(),
        min: 7.0,
        max: 25.0,
        kind: ParameterKind::OddInteger,
    }])
    .expect("valid space");

    let expected: Vec<f64> = vec![7.0, 9.0, 11.0, 13.0, 15.0, 17.0, 19.0, 21.0, 23.0, 25.0];
    let n_odds = expected.len();

    for (i, &exp) in expected.iter().enumerate() {
        let u = i as f64 / (n_odds - 1) as f64;
        let val = space.map_unit_to_physical(0, u);
        assert!(
            (val - exp).abs() < f64::EPSILON,
            "u={u} expected {exp}, got {val}"
        );
    }
}

#[test]
fn map_unit_clamp() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "cycle_time_us".into(),
        min: 0.5,
        max: 2.0,
        kind: ParameterKind::Continuous,
    }])
    .expect("valid space");

    let below = space.map_unit_to_physical(0, -0.5);
    let above = space.map_unit_to_physical(0, 1.5);

    assert!((below - 0.5).abs() < f64::EPSILON);
    assert!((above - 2.0).abs() < f64::EPSILON);
}

#[test]
fn validate_empty_rejected() {
    let result = ParameterSpace::new(vec![]);
    assert!(matches!(result, Err(SensitivityError::EmptyParameterSpace)));
}

#[test]
fn validate_duplicate_rejected() {
    let result = ParameterSpace::new(vec![
        ParameterDef {
            name: "cycle_time_us".into(),
            min: 0.5,
            max: 2.0,
            kind: ParameterKind::Continuous,
        },
        ParameterDef {
            name: "cycle_time_us".into(),
            min: 1.0,
            max: 3.0,
            kind: ParameterKind::Continuous,
        },
    ]);
    assert!(matches!(
        result,
        Err(SensitivityError::DuplicateParameter(ref name)) if name == "cycle_time_us"
    ));
}

#[test]
fn validate_min_ge_max_rejected() {
    let result = ParameterSpace::new(vec![ParameterDef {
        name: "cycle_time_us".into(),
        min: 10.0,
        max: 5.0,
        kind: ParameterKind::Continuous,
    }]);
    assert!(matches!(
        result,
        Err(SensitivityError::InvalidRange { ref name, min, max })
            if name == "cycle_time_us" && (min - 10.0).abs() < f64::EPSILON && (max - 5.0).abs() < f64::EPSILON
    ));
}

#[test]
fn validate_code_distance_bounds() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "code_distance".into(),
        min: 3.0,
        max: 21.0,
        kind: ParameterKind::OddInteger,
    }])
    .expect("valid odd bounds");

    let hw = cultivation_hw();
    assert!(space.validate(&hw).is_ok());
}

#[test]
fn validate_code_distance_even_bound_rejected() {
    let result = ParameterSpace::new(vec![ParameterDef {
        name: "code_distance".into(),
        min: 8.0,
        max: 21.0,
        kind: ParameterKind::OddInteger,
    }]);
    assert!(matches!(result, Err(SensitivityError::NonOddBound { .. })));
}

#[test]
fn validate_against_hw_unknown_param() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "cycle_time_us".into(),
        min: 0.5,
        max: 2.0,
        kind: ParameterKind::Continuous,
    }])
    .expect("valid space");

    let hw = cultivation_hw();
    assert!(space.validate(&hw).is_ok());
}

#[test]
fn validate_against_hw_factory_mismatch() {
    let space = ParameterSpace::new(vec![ParameterDef {
        name: "cycles_per_round".into(),
        min: 5.0,
        max: 20.0,
        kind: ParameterKind::Integer,
    }])
    .expect("valid space");

    let hw = cultivation_hw();
    let result = space.validate(&hw);
    assert!(matches!(
        result,
        Err(SensitivityError::FactoryTypeMismatch { expected, .. }) if expected == "distillation"
    ));
}

#[test]
fn validate_against_hw_routing_mismatch() {
    use pirx_hw::model::{
        BufferConfig, FactoryConfig, HardwareModel, InjectionConfig, MetaConfig, RoutingConfig,
        TimingConfig,
    };
    use pirx_testkit::surface_code_qec;

    let hw = HardwareModel {
        meta: MetaConfig {
            name: "manhattan-test".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: TimingConfig {
            cycle_time_us: 1.0,
            measurement_time_us: 0.5,
            classical_feedback_latency_us: 1.0,
        },
        factory: FactoryConfig::Cultivation {
            count: 1,
            lambda_raw: 0.002,
            fault_distance: 3,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::Manhattan {
            grid_width: 4,
            grid_height: 4,
            cycles_per_hop: 1,
        },
        buffer: BufferConfig {
            capacity: 4,
            preload: 0,
        },
    };

    let space = ParameterSpace::new(vec![ParameterDef {
        name: "overhead_cycles".into(),
        min: 1.0,
        max: 10.0,
        kind: ParameterKind::Integer,
    }])
    .expect("valid space");

    let result = space.validate(&hw);
    assert!(matches!(
        result,
        Err(SensitivityError::RoutingTypeMismatch { expected, .. }) if expected == "scalar"
    ));
}

#[test]
fn map_point_multi_dimensional() {
    let space = ParameterSpace::new(vec![
        ParameterDef {
            name: "physical_error_rate".into(),
            min: 1e-4,
            max: 1e-2,
            kind: ParameterKind::Continuous,
        },
        ParameterDef {
            name: "buffer_capacity".into(),
            min: 2.0,
            max: 10.0,
            kind: ParameterKind::Integer,
        },
    ])
    .expect("valid space");

    let point = space.map_point(&[0.0, 1.0]);
    assert!((point[0] - 1e-4).abs() < 1e-15);
    assert!((point[1] - 10.0).abs() < f64::EPSILON);
}

#[test]
fn validate_negative_min_rejected() {
    let result = ParameterSpace::new(vec![ParameterDef {
        name: "physical_error_rate".into(),
        min: -0.001,
        max: 0.01,
        kind: ParameterKind::Continuous,
    }]);
    assert!(matches!(
        result,
        Err(SensitivityError::NegativeBound { .. })
    ));
}
