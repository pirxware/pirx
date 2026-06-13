//! Magic state factory trait and dispatcher.
//!
//! Factories are pure production-time oracles: given `(current_cycle, rng)`,
//! each returns the future cycle at which one magic state will be ready (or
//! the attempt fails). Buffer management, trace recording, and event
//! scheduling all live in the engine — not here.

mod cultivation;
mod distillation;
mod rz_synthesis;

pub use cultivation::CultivationFactory;
pub use distillation::DistillationFactory;
use pirx_hw::model::{FactoryConfig, QecConfig};
use rand_chacha::ChaCha12Rng;
pub use rz_synthesis::RzSynthesisFactory;

/// Outcome of a single factory production attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn schedule_production(&self, current_cycle: u64, rng: &mut ChaCha12Rng) -> FactoryOutcome;

    /// Human-readable name for reports and trace headers.
    fn name(&self) -> &str;
}

/// Enum dispatch for factory models — zero vtable overhead, inlineable.
///
/// Traits define the contract; enum realizes the dispatch. When a new factory
/// type is implemented, add a variant here and delegate.
pub enum FactoryKind {
    Distillation(DistillationFactory),
    Cultivation(CultivationFactory),
    RzSynthesis(RzSynthesisFactory),
}

impl FactoryModel for FactoryKind {
    fn schedule_production(&self, current_cycle: u64, rng: &mut ChaCha12Rng) -> FactoryOutcome {
        match self {
            Self::Distillation(f) => f.schedule_production(current_cycle, rng),
            Self::Cultivation(f) => f.schedule_production(current_cycle, rng),
            Self::RzSynthesis(f) => f.schedule_production(current_cycle, rng),
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Distillation(f) => f.name(),
            Self::Cultivation(f) => f.name(),
            Self::RzSynthesis(f) => f.name(),
        }
    }
}

/// Create `config.count()` factory instances from already-parsed hardware config.
///
/// `qec.code_distance` is forwarded to cultivation factories, which divide
/// raw exponential service times by the code distance to obtain scheduling cycles.
pub fn create_factories(config: &FactoryConfig, qec: &QecConfig) -> Vec<FactoryKind> {
    match config {
        FactoryConfig::Distillation {
            count,
            cycles_per_round,
            rounds,
            abort_probability,
            ..
        } => (0..*count)
            .map(|_| {
                FactoryKind::Distillation(DistillationFactory {
                    cycles_per_round: *cycles_per_round,
                    rounds: *rounds,
                    abort_probability: *abort_probability,
                })
            })
            .collect(),

        FactoryConfig::Cultivation {
            count, lambda_raw, ..
        } => (0..*count)
            .map(|_| {
                FactoryKind::Cultivation(CultivationFactory {
                    lambda_raw: *lambda_raw,
                    code_distance: qec.code_distance,
                })
            })
            .collect(),

        FactoryConfig::RzSynthesis {
            count,
            mean_cycles_per_state,
            ..
        } => (0..*count)
            .map(|_| {
                FactoryKind::RzSynthesis(RzSynthesisFactory {
                    mean_cycles: *mean_cycles_per_state,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use pirx_hw::model::{DistillationProtocol, FactoryConfig};

    use super::{FactoryModel, create_factories};

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
            protocol: DistillationProtocol::FifteenToOne,
            cycles_per_round: 100,
            rounds: 3,
            abort_probability: 0.0,
        }
    }

    fn rz_synthesis_config(count: u32) -> FactoryConfig {
        FactoryConfig::RzSynthesis {
            count,
            distinct_angles: 1,
            mean_cycles_per_state: 30.0,
        }
    }

    fn qec() -> pirx_hw::model::QecConfig {
        pirx_testkit::surface_code_qec(17)
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

    #[test]
    fn create_rz_synthesis_factories() {
        let factories = create_factories(&rz_synthesis_config(8), &qec());
        assert_eq!(factories.len(), 8);
    }

    #[test]
    fn create_rz_synthesis_names() {
        let factories = create_factories(&rz_synthesis_config(3), &qec());
        assert!(factories.iter().all(|f| f.name() == "rz_synthesis"));
    }
}
