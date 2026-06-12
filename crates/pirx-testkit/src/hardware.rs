//! Shared hardware model fixtures for the pirx workspace.

use pirx_hw::{
    CodeType, RoutingConfig,
    model::{
        BufferConfig, DistillationProtocol, FactoryConfig, HardwareModel, InjectionConfig,
        MetaConfig, QecConfig, TimingConfig,
    },
};
use pirx_ir::circuit::CircuitMetadata;

/// Circuit metadata with zeroed counters. Good enough for any test that
/// doesn't assert on metadata fields.
pub fn blank_meta(name: &str) -> CircuitMetadata {
    CircuitMetadata {
        name: name.into(),
        source_framework: "test".into(),
        t_count: 0,
        clifford_count: 0,
        rotation_count: 0,
        depth: 0,
    }
}

/// Surface code QEC config at the given code distance.
///
/// Physical error rate 10⁻³, threshold 10⁻², prefactor 0.038.
pub fn surface_code_qec(distance: u32) -> QecConfig {
    QecConfig {
        code_type: CodeType::SurfaceCode,
        code_distance: distance,
        physical_error_rate: 1e-3,
        error_correction_threshold: 0.01,
        logical_error_prefactor: 0.038,
    }
}

/// Standard timing: 1 µs cycle, 0.5 µs measurement, 1 µs feedback.
pub fn default_timing() -> TimingConfig {
    TimingConfig {
        cycle_time_us: 1.0,
        measurement_time_us: 0.5,
        classical_feedback_latency_us: 1.0,
    }
}

// ── Hardware model builders ──────────────────────────────────────────────────

/// Single cultivation factory, cold start.
///
/// code_distance=7, λ=0.002, injection p=0.5, fixup_cost=1,
/// buffer capacity=4, preload=0.
pub fn cultivation_hw() -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-cultivation".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
        factory: FactoryConfig::Cultivation {
            count: 1,
            lambda_raw: 0.002,
            fault_distance: 3,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::default(),
        buffer: BufferConfig {
            capacity: 4,
            preload: 0,
        },
    }
}

/// Single distillation factory (15-to-1), cold start.
///
/// 10 cycles/round × 3 rounds, abort p=0.01, code_distance=7,
/// injection p=0.5, fixup_cost=1, buffer capacity=4, preload=0.
pub fn distillation_hw() -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-distillation".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
        factory: FactoryConfig::Distillation {
            count: 1,
            protocol: DistillationProtocol::FifteenToOne,
            cycles_per_round: 10,
            rounds: 3,
            abort_probability: 0.01,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::default(),
        buffer: BufferConfig {
            capacity: 4,
            preload: 0,
        },
    }
}

/// Deterministic distillation: zero abort probability, 18 cycles/round × 3
/// rounds = exactly 54 cycles per magic state. Useful for hand-calculated
/// timing assertions.
pub fn deterministic_distillation_hw(
    factory_count: u32,
    buffer_capacity: u32,
    preload: u32,
) -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-deterministic".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
        factory: FactoryConfig::Distillation {
            count: factory_count,
            protocol: DistillationProtocol::FifteenToOne,
            cycles_per_round: 18,
            rounds: 3,
            abort_probability: 0.0,
        },
        injection: InjectionConfig {
            error_probability: 0.5,
            fixup_cost_cycles: 1,
        },
        routing: RoutingConfig::default(),
        buffer: BufferConfig {
            capacity: buffer_capacity,
            preload,
        },
    }
}

/// Cultivation factory with Manhattan routing on a `width × height` grid.
pub fn manhattan_hw(width: u32, height: u32) -> HardwareModel {
    HardwareModel {
        meta: MetaConfig {
            name: "test-manhattan".into(),
            description: String::new(),
        },
        qec: surface_code_qec(7),
        timing: default_timing(),
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
            grid_width: width,
            grid_height: height,
            cycles_per_hop: 1,
        },
        buffer: BufferConfig {
            capacity: 4,
            preload: 0,
        },
    }
}
