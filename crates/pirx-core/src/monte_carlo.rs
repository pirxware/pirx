//! Monte Carlo simulation mode — run N independent replicas, extract scalar
//! summaries, aggregate into distribution statistics.
//!
//! Each replica constructs its own [`Engine`] with a deterministic seed
//! (`base_seed + i`), runs to completion, extracts a lightweight
//! [`ReplicaSummary`], and discards the trace. Peak memory: one trace per
//! thread, not N traces.

use pirx_hw::model::HardwareModel;
use pirx_ir::ValidatedCircuit;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    engine::{Engine, EngineConfig, EngineError},
    trace::{Trace, TraceEventKind},
};

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors from Monte Carlo simulation.
#[derive(Debug, Error)]
pub enum MonteCarloError {
    #[error("engine error: {0}")]
    Engine(#[from] EngineError),

    #[cfg(not(target_arch = "wasm32"))]
    #[error("thread pool build failed ({threads} threads): {reason}")]
    ThreadPool { threads: usize, reason: String },
}

// ── Configuration ────────────────────────────────────────────────────────────

/// Configuration for Monte Carlo simulation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MonteCarloConfig {
    /// Number of independent replicas. Clamped to minimum 1.
    pub replicas: u32,
    /// Base seed. Replica `i` uses `base_seed.wrapping_add(i)`.
    pub base_seed: u64,
    /// Maximum cycles per replica (`None` = run to completion).
    pub max_cycles: Option<u64>,
    /// Number of rayon threads (`None` = rayon default = num CPUs).
    /// Ignored on WASM targets.
    pub threads: Option<usize>,
}

// ── Per-replica summary ──────────────────────────────────────────────────────

/// Scalar statistics extracted from a single simulation replica.
///
/// Cheap to store (< 200 bytes). Computed via a single O(n) pass over the
/// trace, then the trace is discarded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSummary {
    /// Seed used for this replica.
    pub seed: u64,
    /// Total simulation cycles.
    pub total_cycles: u64,
    /// Whether the simulation was truncated by `max_cycles`.
    pub truncated: bool,
    /// Number of T-gate stall events (`GateServed` with `wait > 0`).
    pub stall_count: u64,
    /// Total cycles spent stalling (sum of all wait durations).
    pub total_stall_cycles: u64,
    /// Maximum single-gate stall duration.
    pub max_stall_cycles: u64,
    /// Number of injection errors.
    pub injection_errors: u64,
    /// Number of fixups inserted.
    pub fixups_inserted: u64,
    /// Mean factory utilization (fraction of time factories are active).
    pub mean_factory_utilization: f64,
    /// Number of `BufferFull` events (factory produced but buffer was at capacity).
    pub buffer_full_events: u64,
    /// Total magic states consumed during this replica.
    pub magic_states_consumed: u64,
    /// Total accumulated infidelity: `magic_states_consumed × p_logical`.
    pub total_infidelity: f64,
}

// ── Distribution statistics ──────────────────────────────────────────────────

/// Distribution summary over Monte Carlo replicas for a single metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    pub mean: f64,
    pub stddev: f64,
    pub min: f64,
    pub max: f64,
    pub p5: f64,
    pub p25: f64,
    pub median: f64,
    pub p75: f64,
    pub p95: f64,
}

// ── Aggregate result ─────────────────────────────────────────────────────────

/// Complete Monte Carlo simulation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonteCarloResult {
    /// Per-replica summaries (length = `config.replicas`).
    pub replicas: Vec<ReplicaSummary>,
    /// Distribution of total simulation cycles.
    pub total_cycles: Distribution,
    /// Distribution of stall counts.
    pub stall_count: Distribution,
    /// Distribution of total stall cycles.
    pub total_stall_cycles: Distribution,
    /// Distribution of injection error counts.
    pub injection_errors: Distribution,
    /// Distribution of fixup counts.
    pub fixups_inserted: Distribution,
    /// Distribution of maximum single-gate stall duration.
    pub max_stall_cycles: Distribution,
    /// Distribution of mean factory utilization.
    pub mean_factory_utilization: Distribution,
    /// Distribution of buffer-full event counts.
    pub buffer_full_events: Distribution,
    /// Distribution of magic states consumed.
    pub magic_states_consumed: Distribution,
    /// Distribution of total accumulated infidelity.
    pub total_infidelity: Distribution,
    /// Number of replicas truncated by `max_cycles`.
    pub truncated_count: u32,
    /// Configuration used for this run.
    pub config: MonteCarloConfig,
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Run `config.replicas` independent simulations, extract per-replica
/// summaries, and aggregate into distribution statistics.
///
/// Each replica gets seed `config.base_seed.wrapping_add(i)`. Replicas
/// are run in parallel via rayon on non-WASM targets.
pub fn run_monte_carlo(
    circuit: &ValidatedCircuit,
    hw: &HardwareModel,
    config: MonteCarloConfig,
) -> Result<MonteCarloResult, MonteCarloError> {
    let replicas = config.replicas.max(1);
    #[allow(clippy::cast_possible_truncation)]
    let factory_count = hw.factory.count().min(u32::from(u16::MAX)) as u16;

    // Preflight: validate engine construction once before spawning threads.
    let preflight_config = EngineConfig {
        seed: config.base_seed,
        max_cycles: config.max_cycles,
    };
    // Construct and immediately drop — validates inputs without keeping state.
    Engine::new(circuit, hw, preflight_config).map(drop)?;

    let summaries = collect_summaries(replicas, circuit, hw, &config, factory_count)?;

    Ok(aggregate(summaries, config))
}

// ── Parallel / sequential dispatch ───────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn collect_summaries(
    replicas: u32,
    circuit: &ValidatedCircuit,
    hw: &HardwareModel,
    config: &MonteCarloConfig,
    factory_count: u16,
) -> Result<Vec<ReplicaSummary>, MonteCarloError> {
    let pool = match config.threads {
        Some(n) => Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .map_err(|e| MonteCarloError::ThreadPool {
                    threads: n,
                    reason: e.to_string(),
                })?,
        ),
        None => None,
    };

    let run = || -> Result<Vec<ReplicaSummary>, MonteCarloError> {
        (0..replicas)
            .into_par_iter()
            .map(|i| {
                let seed = config.base_seed.wrapping_add(u64::from(i));
                let engine_config = EngineConfig {
                    seed,
                    max_cycles: config.max_cycles,
                };
                let engine = Engine::new(circuit, hw, engine_config)?;
                let trace = engine.run();
                Ok(extract_summary(&trace, seed, factory_count))
            })
            .collect()
    };

    match &pool {
        Some(p) => p.install(run),
        None => run(),
    }
}

#[cfg(target_arch = "wasm32")]
fn collect_summaries(
    replicas: u32,
    circuit: &ValidatedCircuit,
    hw: &HardwareModel,
    config: &MonteCarloConfig,
    factory_count: u16,
) -> Result<Vec<ReplicaSummary>, MonteCarloError> {
    (0..replicas)
        .map(|i| {
            let seed = config.base_seed.wrapping_add(u64::from(i));
            let engine_config = EngineConfig {
                seed,
                max_cycles: config.max_cycles,
            };
            let engine = Engine::new(circuit, hw, engine_config)?;
            let trace = engine.run();
            Ok(extract_summary(&trace, seed, factory_count))
        })
        .collect()
}

// ── Summary extraction ───────────────────────────────────────────────────────

/// Single O(n) pass over trace events to extract scalar statistics.
/// No per-bucket vectors — only running counters.
fn extract_summary(trace: &Trace, seed: u64, factory_count: u16) -> ReplicaSummary {
    let mut stall_count: u64 = 0;
    let mut total_stall_cycles: u64 = 0;
    let mut max_stall_cycles: u64 = 0;
    let mut injection_errors: u64 = 0;
    let mut fixups_inserted: u64 = 0;
    let mut buffer_full_events: u64 = 0;

    let mut total_factory_active_cycles: u64 = 0;
    let mut factory_starts: Vec<Option<u64>> = vec![None; usize::from(factory_count)];

    for event in &trace.events {
        match &event.kind {
            TraceEventKind::GateServed { wait, .. } if *wait > 0 => {
                let w = u64::from(*wait);
                stall_count += 1;
                total_stall_cycles = total_stall_cycles.saturating_add(w);
                max_stall_cycles = max_stall_cycles.max(w);
            }
            TraceEventKind::GateServed { .. } => {}
            TraceEventKind::InjectionError { .. } => injection_errors += 1,
            TraceEventKind::FixupInserted { .. } => fixups_inserted += 1,
            TraceEventKind::BufferFull => buffer_full_events += 1,
            TraceEventKind::FactoryStarted { factory_id } => {
                if let Some(slot) = factory_starts.get_mut(usize::from(*factory_id)) {
                    *slot = Some(event.cycle);
                }
            }
            TraceEventKind::FactoryProduced { factory_id }
            | TraceEventKind::FactoryFailed { factory_id } => {
                if let Some(slot) = factory_starts.get_mut(usize::from(*factory_id))
                    && let Some(start) = slot.take()
                {
                    total_factory_active_cycles = total_factory_active_cycles
                        .saturating_add(event.cycle.saturating_sub(start));
                }
            }
            TraceEventKind::GateReady { .. }
            | TraceEventKind::GateScheduled { .. }
            | TraceEventKind::GateStalled { .. }
            | TraceEventKind::GateCompleted { .. }
            | TraceEventKind::FixupCompleted { .. }
            | TraceEventKind::BufferEnqueue { .. }
            | TraceEventKind::BufferDequeue { .. }
            | TraceEventKind::RoutingStarted { .. }
            | TraceEventKind::RoutingCompleted { .. }
            | TraceEventKind::MeasurementOutcome { .. }
            | TraceEventKind::OpsActivated { .. } => {}
        }
    }

    let mean_utilization = if trace.total_cycles > 0 && factory_count > 0 {
        total_factory_active_cycles as f64 / (trace.total_cycles as f64 * f64::from(factory_count))
    } else {
        0.0
    };

    ReplicaSummary {
        seed,
        total_cycles: trace.total_cycles,
        truncated: trace.truncated,
        stall_count,
        total_stall_cycles,
        max_stall_cycles,
        injection_errors,
        fixups_inserted,
        mean_factory_utilization: mean_utilization,
        buffer_full_events,
        magic_states_consumed: trace.magic_states_consumed,
        total_infidelity: trace.magic_states_consumed as f64 * trace.p_logical,
    }
}

// ── Aggregation ──────────────────────────────────────────────────────────────

fn aggregate(summaries: Vec<ReplicaSummary>, config: MonteCarloConfig) -> MonteCarloResult {
    #[allow(clippy::cast_possible_truncation)]
    let truncated_count = summaries.iter().filter(|s| s.truncated).count() as u32;

    let total_cycles = distribution_from(&summaries, |s| s.total_cycles as f64);
    let stall_count = distribution_from(&summaries, |s| s.stall_count as f64);
    let total_stall_cycles = distribution_from(&summaries, |s| s.total_stall_cycles as f64);
    let max_stall_cycles = distribution_from(&summaries, |s| s.max_stall_cycles as f64);
    let injection_errors = distribution_from(&summaries, |s| s.injection_errors as f64);
    let fixups_inserted = distribution_from(&summaries, |s| s.fixups_inserted as f64);
    let mean_factory_utilization = distribution_from(&summaries, |s| s.mean_factory_utilization);
    let buffer_full_events = distribution_from(&summaries, |s| s.buffer_full_events as f64);
    let magic_states_consumed = distribution_from(&summaries, |s| s.magic_states_consumed as f64);
    let total_infidelity = distribution_from(&summaries, |s| s.total_infidelity);

    MonteCarloResult {
        replicas: summaries,
        total_cycles,
        stall_count,
        total_stall_cycles,
        max_stall_cycles,
        injection_errors,
        fixups_inserted,
        mean_factory_utilization,
        buffer_full_events,
        magic_states_consumed,
        total_infidelity,
        truncated_count,
        config,
    }
}

fn distribution_from(
    summaries: &[ReplicaSummary],
    f: impl Fn(&ReplicaSummary) -> f64,
) -> Distribution {
    let mut values: Vec<f64> = summaries.iter().map(&f).collect();
    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = values.len();
    if n == 0 {
        return Distribution {
            mean: 0.0,
            stddev: 0.0,
            min: 0.0,
            max: 0.0,
            p5: 0.0,
            p25: 0.0,
            median: 0.0,
            p75: 0.0,
            p95: 0.0,
        };
    }

    let mean = values.iter().sum::<f64>() / n as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;

    Distribution {
        mean,
        stddev: variance.sqrt(),
        min: values.first().copied().unwrap_or(0.0),
        max: values.last().copied().unwrap_or(0.0),
        p5: percentile(&values, 0.05),
        p25: percentile(&values, 0.25),
        median: percentile(&values, 0.50),
        p75: percentile(&values, 0.75),
        p95: percentile(&values, 0.95),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let idx = (p * (sorted.len() - 1) as f64).round() as usize;
    sorted
        .get(idx.min(sorted.len() - 1))
        .copied()
        .unwrap_or(0.0)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::trace::{TraceCollector, TraceEventKind};

    // ── Unit: percentile ─────────────────────────────────────────────────────

    #[test]
    fn percentile_empty_returns_zero() {
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn percentile_single_value() {
        assert_eq!(percentile(&[42.0], 0.0), 42.0);
        assert_eq!(percentile(&[42.0], 0.5), 42.0);
        assert_eq!(percentile(&[42.0], 1.0), 42.0);
    }

    #[test]
    fn percentile_known_values() {
        let sorted = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&sorted, 0.0), 1.0);
        assert_eq!(percentile(&sorted, 0.5), 3.0);
        assert_eq!(percentile(&sorted, 1.0), 5.0);
    }

    // ── Unit: distribution_from ──────────────────────────────────────────────

    #[test]
    fn distribution_single_value() {
        let summaries = vec![replica_summary(42, 100)];
        let dist = distribution_from(&summaries, |s| s.total_cycles as f64);
        assert_eq!(dist.mean, 100.0);
        assert_eq!(dist.stddev, 0.0);
        assert_eq!(dist.min, 100.0);
        assert_eq!(dist.max, 100.0);
        assert_eq!(dist.median, 100.0);
        assert_eq!(dist.p5, 100.0);
        assert_eq!(dist.p95, 100.0);
    }

    #[test]
    fn distribution_known_values() {
        let summaries: Vec<ReplicaSummary> = (1..=5).map(|i| replica_summary(i, i * 100)).collect();
        let dist = distribution_from(&summaries, |s| s.total_cycles as f64);
        assert!((dist.mean - 300.0).abs() < f64::EPSILON);
        assert_eq!(dist.min, 100.0);
        assert_eq!(dist.max, 500.0);
        assert_eq!(dist.median, 300.0);
        assert_eq!(dist.p5, 100.0);
        assert_eq!(dist.p95, 500.0);
    }

    // ── Unit: extract_summary ────────────────────────────────────────────────

    #[test]
    fn extract_summary_empty_trace() {
        let collector = TraceCollector::new(0);
        let trace = collector.finish(0, 0, 0.0, 0);
        let summary = extract_summary(&trace, 0, 1);

        assert_eq!(summary.stall_count, 0);
        assert_eq!(summary.injection_errors, 0);
        assert_eq!(summary.fixups_inserted, 0);
        assert_eq!(summary.buffer_full_events, 0);
        assert_eq!(summary.total_cycles, 0);
        assert_eq!(summary.mean_factory_utilization, 0.0);
        assert_eq!(summary.magic_states_consumed, 0);
        assert_eq!(summary.total_infidelity, 0.0);
    }

    #[test]
    fn extract_summary_counts_stalls() {
        let mut collector = TraceCollector::new(4);
        collector.record(10, TraceEventKind::GateServed { gate: 1, wait: 5 });
        collector.record(20, TraceEventKind::GateServed { gate: 2, wait: 10 });
        collector.record(30, TraceEventKind::GateServed { gate: 3, wait: 0 });
        let trace = collector.finish(42, 30, 0.0, 3);
        let summary = extract_summary(&trace, 42, 1);

        assert_eq!(summary.stall_count, 2);
        assert_eq!(summary.total_stall_cycles, 15);
        assert_eq!(summary.max_stall_cycles, 10);
        assert_eq!(summary.magic_states_consumed, 3);
    }

    #[test]
    fn extract_summary_factory_utilization() {
        let mut collector = TraceCollector::new(4);
        // Factory 0 active from cycle 0 to cycle 50.
        collector.record(0, TraceEventKind::FactoryStarted { factory_id: 0 });
        collector.record(50, TraceEventKind::FactoryProduced { factory_id: 0 });
        let trace = collector.finish(0, 100, 0.0, 0);
        let summary = extract_summary(&trace, 0, 1);

        // 50 active cycles out of 100 total, 1 factory → 0.5.
        assert!((summary.mean_factory_utilization - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_summary_counts_injection_and_fixup() {
        let mut collector = TraceCollector::new(4);
        collector.record(10, TraceEventKind::InjectionError { gate: 1 });
        collector.record(
            10,
            TraceEventKind::FixupInserted {
                fixup: 1000,
                original: 1,
            },
        );
        collector.record(20, TraceEventKind::InjectionError { gate: 2 });
        collector.record(
            20,
            TraceEventKind::FixupInserted {
                fixup: 1001,
                original: 2,
            },
        );
        let trace = collector.finish(0, 20, 0.0, 0);
        let summary = extract_summary(&trace, 0, 1);

        assert_eq!(summary.injection_errors, 2);
        assert_eq!(summary.fixups_inserted, 2);
    }

    #[test]
    fn extract_summary_counts_buffer_full() {
        let mut collector = TraceCollector::new(4);
        collector.record(10, TraceEventKind::BufferFull);
        collector.record(20, TraceEventKind::BufferFull);
        collector.record(30, TraceEventKind::BufferFull);
        let trace = collector.finish(0, 30, 0.0, 0);
        let summary = extract_summary(&trace, 0, 1);

        assert_eq!(summary.buffer_full_events, 3);
    }

    // ── Helper ───────────────────────────────────────────────────────────────

    fn replica_summary(seed: u64, total_cycles: u64) -> ReplicaSummary {
        ReplicaSummary {
            seed,
            total_cycles,
            truncated: false,
            stall_count: 0,
            total_stall_cycles: 0,
            max_stall_cycles: 0,
            injection_errors: 0,
            fixups_inserted: 0,
            mean_factory_utilization: 0.0,
            buffer_full_events: 0,
            magic_states_consumed: 0,
            total_infidelity: 0.0,
        }
    }
}
