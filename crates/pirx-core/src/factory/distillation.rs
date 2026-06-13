//! Distillation factory model.
//!
//! Checks for an abort at each round (probability `abort_probability`).
//! First abort → `Failed` at that round's completion cycle.
//! All rounds clear → `Produced` at `current_cycle + cycles_per_round * rounds`.

use rand::Rng as _;
use rand_chacha::ChaCha12Rng;

use super::{FactoryModel, FactoryOutcome};

/// Magic state factory based on probabilistic distillation rounds.
///
/// Each of `rounds` rounds independently samples an abort check. The first
/// aborting round ends production early; the engine restarts the factory
/// immediately after `Failed`. All rounds clearing → `Produced`.
#[derive(Debug, Clone)]
pub struct DistillationFactory {
    pub cycles_per_round: u32,
    pub rounds: u32,
    /// Per-round probability of aborting the entire distillation attempt.
    pub abort_probability: f64,
}

impl FactoryModel for DistillationFactory {
    fn schedule_production(&self, current_cycle: u64, rng: &mut ChaCha12Rng) -> FactoryOutcome {
        let total_cycles = u64::from(self.cycles_per_round).saturating_mul(u64::from(self.rounds));
        for round in 0..self.rounds {
            if rng.random::<f64>() < self.abort_probability {
                let failure_cycle = current_cycle.saturating_add(
                    u64::from(self.cycles_per_round).saturating_mul(u64::from(round + 1)),
                );
                return FactoryOutcome::Failed { failure_cycle };
            }
        }
        FactoryOutcome::Produced {
            completion_cycle: current_cycle.saturating_add(total_cycles),
        }
    }

    fn name(&self) -> &str {
        "distillation"
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use rand::SeedableRng;
    use rand_chacha::ChaCha12Rng;

    use super::{DistillationFactory, FactoryModel, FactoryOutcome};

    fn no_abort() -> DistillationFactory {
        DistillationFactory {
            cycles_per_round: 100,
            rounds: 3,
            abort_probability: 0.0,
        }
    }

    fn always_aborts() -> DistillationFactory {
        DistillationFactory {
            cycles_per_round: 100,
            rounds: 3,
            abort_probability: 1.0,
        }
    }

    #[test]
    fn distillation_deterministic_no_abort() {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        // abort_probability = 0.0: all rounds pass, completion = 0 + 100 * 3 = 300
        let outcome = no_abort().schedule_production(0, &mut rng);
        assert!(matches!(
            outcome,
            FactoryOutcome::Produced {
                completion_cycle: 300
            }
        ));
    }

    #[test]
    fn distillation_deterministic_no_abort_offset_cycle() {
        let mut rng = ChaCha12Rng::seed_from_u64(42);
        // Same logic with current_cycle = 500: completion = 500 + 300 = 800
        let outcome = no_abort().schedule_production(500, &mut rng);
        assert!(matches!(
            outcome,
            FactoryOutcome::Produced {
                completion_cycle: 800
            }
        ));
    }

    #[test]
    fn distillation_always_aborts() {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        // abort_probability = 1.0: round 0 always aborts, failure_cycle = 0 + 100 * 1 = 100
        let outcome = always_aborts().schedule_production(0, &mut rng);
        assert!(matches!(
            outcome,
            FactoryOutcome::Failed { failure_cycle: 100 }
        ));
    }

    #[test]
    fn distillation_always_aborts_offset_cycle() {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        let outcome = always_aborts().schedule_production(200, &mut rng);
        assert!(matches!(
            outcome,
            FactoryOutcome::Failed { failure_cycle: 300 }
        ));
    }

    #[test]
    fn distillation_name() {
        assert_eq!(no_abort().name(), "distillation");
    }
}
