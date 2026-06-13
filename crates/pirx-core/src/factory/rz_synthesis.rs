//! Rz synthesis factory model.
//!
//! Rotation state production time is exponentially distributed with
//! rate `1 / mean_cycles`. Identical in structure to cultivation but
//! parameterised by user-specified mean, not `lambda_raw / code_distance`.

use rand_chacha::ChaCha12Rng;
use rand_distr::{Distribution, Exp};

use super::{FactoryModel, FactoryOutcome};

/// Magic state factory for arbitrary-angle rotation synthesis.
///
/// Models the repeat-until-success (RUS) synthesis protocol as an
/// exponential service time. Production always succeeds (no abort) —
/// RUS failures are absorbed into the service time distribution.
#[derive(Debug, Clone)]
pub struct RzSynthesisFactory {
    /// Mean production time in QEC cycles. Must be > 0.
    pub mean_cycles: f64,
}

impl FactoryModel for RzSynthesisFactory {
    fn schedule_production(&self, current_cycle: u64, rng: &mut ChaCha12Rng) -> FactoryOutcome {
        let Ok(dist) = Exp::new(1.0 / self.mean_cycles) else {
            return FactoryOutcome::Produced {
                completion_cycle: current_cycle + 1,
            };
        };
        let raw_time: f64 = dist.sample(rng);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let cycles = raw_time.ceil() as u64;
        FactoryOutcome::Produced {
            completion_cycle: current_cycle.saturating_add(cycles.max(1)),
        }
    }

    fn name(&self) -> &str {
        "rz_synthesis"
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests {
    use rand::SeedableRng;
    use rand_chacha::ChaCha12Rng;

    use super::{FactoryModel, FactoryOutcome, RzSynthesisFactory};

    fn factory() -> RzSynthesisFactory {
        RzSynthesisFactory { mean_cycles: 30.0 }
    }

    #[test]
    fn rz_synthesis_produces_positive_cycle() {
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        let outcome = factory().schedule_production(100, &mut rng);
        assert!(matches!(outcome, FactoryOutcome::Produced { .. }));
        if let FactoryOutcome::Produced { completion_cycle } = outcome {
            assert!(
                completion_cycle > 100,
                "completion must be after current cycle"
            );
        }
    }

    #[test]
    fn rz_synthesis_determinism() {
        let f = factory();
        let o1 = f.schedule_production(0, &mut ChaCha12Rng::seed_from_u64(99));
        let o2 = f.schedule_production(0, &mut ChaCha12Rng::seed_from_u64(99));
        assert_eq!(o1, o2);
    }

    #[test]
    fn rz_synthesis_min_one_cycle() {
        let f = RzSynthesisFactory { mean_cycles: 0.001 };
        let mut rng = ChaCha12Rng::seed_from_u64(0);
        for _ in 0..100 {
            let outcome = f.schedule_production(0, &mut rng);
            assert!(matches!(outcome, FactoryOutcome::Produced { .. }));
            if let FactoryOutcome::Produced { completion_cycle } = outcome {
                assert!(completion_cycle >= 1);
            }
        }
    }

    #[test]
    fn rz_synthesis_name() {
        assert_eq!(factory().name(), "rz_synthesis");
    }

    #[test]
    fn rz_synthesis_mean_convergence() {
        let f = factory();
        let mut rng = ChaCha12Rng::seed_from_u64(42);
        let n = 10_000;
        let total: f64 = (0..n)
            .map(|_| {
                if let FactoryOutcome::Produced { completion_cycle } =
                    f.schedule_production(0, &mut rng)
                {
                    completion_cycle as f64
                } else {
                    0.0
                }
            })
            .sum();
        let sample_mean = total / f64::from(n);
        let relative_error = (sample_mean - f.mean_cycles).abs() / f.mean_cycles;
        assert!(
            relative_error < 0.05,
            "sample mean {sample_mean:.2} should be within 5% of expected {:.2}, \
             relative error: {relative_error:.4}",
            f.mean_cycles
        );
    }
}
