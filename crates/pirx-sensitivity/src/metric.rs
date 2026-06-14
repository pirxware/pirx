//! Metric extraction from simulation traces.

use pirx_core::ReplicaSummary;
use serde::{Deserialize, Serialize};

/// Which scalar metric to extract from a simulation replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMetric {
    TotalCycles,
    TotalStallCycles,
    StallCount,
    MaxStallCycles,
    MeanFactoryUtilization,
    TotalInfidelity,
    MagicStatesConsumed,
}

impl OutputMetric {
    /// Extract this metric's value from a replica summary as `f64`.
    #[allow(clippy::cast_precision_loss)]
    pub fn extract(&self, summary: &ReplicaSummary) -> f64 {
        match self {
            Self::TotalCycles => summary.total_cycles as f64,
            Self::TotalStallCycles => summary.total_stall_cycles as f64,
            Self::StallCount => summary.stall_count as f64,
            Self::MaxStallCycles => summary.max_stall_cycles as f64,
            Self::MeanFactoryUtilization => summary.mean_factory_utilization,
            Self::TotalInfidelity => summary.total_infidelity,
            Self::MagicStatesConsumed => summary.magic_states_consumed as f64,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn test_summary() -> ReplicaSummary {
        ReplicaSummary {
            seed: 0,
            total_cycles: 500,
            truncated: false,
            stall_count: 10,
            total_stall_cycles: 120,
            max_stall_cycles: 30,
            injection_errors: 3,
            fixups_inserted: 3,
            mean_factory_utilization: 0.75,
            buffer_full_events: 2,
            magic_states_consumed: 42,
            total_infidelity: 0.001,
        }
    }

    #[test]
    fn output_metric_extract_total_cycles() {
        let summary = test_summary();
        let value = OutputMetric::TotalCycles.extract(&summary);
        assert!((value - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_extract_total_stall_cycles() {
        let summary = test_summary();
        let value = OutputMetric::TotalStallCycles.extract(&summary);
        assert!((value - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_extract_stall_count() {
        let summary = test_summary();
        let value = OutputMetric::StallCount.extract(&summary);
        assert!((value - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_extract_max_stall_cycles() {
        let summary = test_summary();
        let value = OutputMetric::MaxStallCycles.extract(&summary);
        assert!((value - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_extract_mean_factory_utilization() {
        let summary = test_summary();
        let value = OutputMetric::MeanFactoryUtilization.extract(&summary);
        assert!((value - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_extract_total_infidelity() {
        let summary = test_summary();
        let value = OutputMetric::TotalInfidelity.extract(&summary);
        assert!((value - 0.001).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_extract_magic_states_consumed() {
        let summary = test_summary();
        let value = OutputMetric::MagicStatesConsumed.extract(&summary);
        assert!((value - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn output_metric_serde_roundtrip() {
        let metric = OutputMetric::TotalCycles;
        let json = serde_json::to_string(&metric).unwrap();
        assert_eq!(json, "\"total_cycles\"");
        let back: OutputMetric = serde_json::from_str(&json).unwrap();
        assert_eq!(back, metric);
    }
}
