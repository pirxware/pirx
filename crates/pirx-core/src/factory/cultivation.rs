//! Cultivation factory model.
//!
//! Magic state preparation time is exponentially distributed with rate
//! `lambda_raw`. The raw sample is divided by `code_distance` (cycles per
//! raw time unit) and rounded up — ensuring at least 1 scheduling cycle.

use rand::rngs::StdRng;
use rand_distr::{Distribution, Exp};
use serde::{Deserialize, Serialize};

use super::{FactoryModel, FactoryOutcome};

/// Magic state factory modelling cultivation as an exponential service time.
///
/// `lambda_raw` must be strictly positive. A non-positive `lambda_raw` (invalid
/// hardware config) results in a 1-cycle fallback so the engine keeps running;
/// the anomalous speed is visible in the trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CultivationFactory {
    /// Exponential rate parameter (1 / mean raw service time). Must be > 0.
    pub lambda_raw: f64,
    /// QEC code distance; raw service time is divided by this to get cycles.
    pub code_distance: u32,
}

impl FactoryModel for CultivationFactory {
    fn schedule_production(&self, current_cycle: u64, rng: &mut StdRng) -> FactoryOutcome {
        let Ok(dist) = Exp::new(self.lambda_raw) else {
            // lambda_raw <= 0: invalid config that slipped validation.
            // 1-cycle fallback keeps the engine alive; trace shows impossible speed.
            return FactoryOutcome::Produced {
                completion_cycle: current_cycle + 1,
            };
        };
        let raw_time: f64 = dist.sample(rng);
        // Exp samples are non-negative; ceil gives an integer >= 0. Cast is safe
        // because realistic scheduling cycles fit well within u64.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let scheduling_cycles = (raw_time / f64::from(self.code_distance)).ceil() as u64;
        FactoryOutcome::Produced {
            completion_cycle: current_cycle + scheduling_cycles.max(1),
        }
    }

    fn name(&self) -> &str {
        "cultivation"
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use rand::{SeedableRng, rngs::StdRng};

    use super::{CultivationFactory, FactoryModel, FactoryOutcome};

    fn factory() -> CultivationFactory {
        CultivationFactory {
            lambda_raw: 0.002,
            code_distance: 17,
        }
    }

    #[test]
    fn cultivation_produces_positive_cycle() {
        let mut rng = StdRng::seed_from_u64(0);
        let outcome = factory().schedule_production(100, &mut rng);
        // Cultivation always produces (exponential distribution is non-negative)
        assert!(matches!(outcome, FactoryOutcome::Produced { .. }));
        if let FactoryOutcome::Produced { completion_cycle } = outcome {
            assert!(
                completion_cycle > 100,
                "completion must be after current cycle"
            );
        }
    }

    #[test]
    fn cultivation_determinism() {
        let f = factory();
        let o1 = f.schedule_production(0, &mut StdRng::seed_from_u64(99));
        let o2 = f.schedule_production(0, &mut StdRng::seed_from_u64(99));
        assert_eq!(o1, o2);
    }

    #[test]
    fn cultivation_min_one_cycle() {
        // Even with a very fast factory (large lambda), completion >= current_cycle + 1.
        let f = CultivationFactory {
            lambda_raw: 1_000_000.0,
            code_distance: 17,
        };
        let mut rng = StdRng::seed_from_u64(0);
        for _ in 0..100 {
            let outcome = f.schedule_production(0, &mut rng);
            assert!(matches!(outcome, FactoryOutcome::Produced { .. }));
            if let FactoryOutcome::Produced { completion_cycle } = outcome {
                assert!(completion_cycle >= 1);
            }
        }
    }

    #[test]
    fn cultivation_name() {
        assert_eq!(factory().name(), "cultivation");
    }
}
