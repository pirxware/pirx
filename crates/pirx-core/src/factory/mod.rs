//! Magic state factory trait and dispatcher.
//!
//! Factories are pure production-time oracles: given `(current_cycle, rng)`,
//! each returns the future cycle at which one magic state will be ready (or
//! the attempt fails). Buffer management, trace recording, and event
//! scheduling all live in the engine — not here.

mod cultivation;
mod distillation;

pub use cultivation::CultivationFactory;
pub use distillation::DistillationFactory;

use pirx_hw::model::{FactoryConfig, QecConfig};
use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};

/// Outcome of a single factory production attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactoryOutcome {
    /// Production succeeds at `completion_cycle`.
    Produced { completion_cycle: u64 },
    /// Production fails at `failure_cycle` (abort/restart needed).
    Failed { failure_cycle: u64 },
}

/// A magic state factory model.
///
/// Implementations are stateless after construction — all stochastic state
/// flows through the explicit `rng` parameter. `Send` is required so the
/// engine can be moved across threads in a parallel sensitivity sweep.
pub trait FactoryModel: Send {
    /// Schedule the next production event.
    ///
    /// Returns the future cycle at which production completes or fails.
    /// The caller (engine) schedules the corresponding `EngineEvent` and
    /// restarts the factory after each outcome. The factory never touches
    /// the buffer, emits trace events, or inspects the event queue.
    fn schedule_production(&self, current_cycle: u64, rng: &mut StdRng) -> FactoryOutcome;

    /// Human-readable name for reports and trace headers.
    fn name(&self) -> &str;
}

/// Create `config.count()` factory instances from already-parsed hardware config.
///
/// `qec.code_distance` is forwarded to cultivation factories, which divide
/// raw exponential service times by the code distance to obtain scheduling cycles.
pub fn create_factories(config: &FactoryConfig, qec: &QecConfig) -> Vec<Box<dyn FactoryModel>> {
    match config {
        FactoryConfig::Distillation {
            count,
            cycles_per_round,
            rounds,
            abort_probability,
            ..
        } => (0..*count)
            .map(|_| {
                Box::new(DistillationFactory {
                    cycles_per_round: *cycles_per_round,
                    rounds: *rounds,
                    abort_probability: *abort_probability,
                }) as Box<dyn FactoryModel>
            })
            .collect(),

        FactoryConfig::Cultivation {
            count, lambda_raw, ..
        } => (0..*count)
            .map(|_| {
                Box::new(CultivationFactory {
                    lambda_raw: *lambda_raw,
                    code_distance: qec.code_distance,
                }) as Box<dyn FactoryModel>
            })
            .collect(),

        FactoryConfig::RzSynthesis { .. } => todo!("RzSynthesis factory not yet implemented"),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use pirx_hw::model::{FactoryConfig, QecConfig};

    use super::create_factories;

    fn cultivation_config(count: u32) -> FactoryConfig {
        FactoryConfig::Cultivation {
            count,
            lambda_raw: 0.002,
            fault_distance: 3,
        }
    }

    fn distillation_config(count: u32) -> FactoryConfig {
        FactoryConfig::Distillation {
            count,
            protocol: "15-to-1".to_owned(),
            cycles_per_round: 100,
            rounds: 3,
            abort_probability: 0.0,
        }
    }

    fn qec() -> QecConfig {
        QecConfig {
            code_type: "surface_code".to_owned(),
            code_distance: 17,
            physical_error_rate: 1e-3,
            error_correction_threshold: 0.01,
            logical_error_prefactor: 0.038,
        }
    }

    #[test]
    fn create_factories_count() {
        let distillation = create_factories(&distillation_config(3), &qec());
        assert_eq!(distillation.len(), 3);

        let cultivation = create_factories(&cultivation_config(5), &qec());
        assert_eq!(cultivation.len(), 5);
    }

    #[test]
    fn create_factories_zero_count() {
        let factories = create_factories(&cultivation_config(0), &qec());
        assert!(factories.is_empty());
    }

    #[test]
    fn create_factories_names() {
        let factories = create_factories(&distillation_config(2), &qec());
        assert!(factories.iter().all(|f| f.name() == "distillation"));

        let factories = create_factories(&cultivation_config(2), &qec());
        assert!(factories.iter().all(|f| f.name() == "cultivation"));
    }
}
