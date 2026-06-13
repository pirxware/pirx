//! Python wrappers for Monte Carlo types.

use pyo3::prelude::*;

/// Distribution statistics over Monte Carlo replicas (frozen).
#[pyclass(frozen, name = "Distribution")]
pub struct PyDistribution {
    pub(crate) inner: pirx_core::Distribution,
}

#[pymethods]
impl PyDistribution {
    #[getter]
    fn mean(&self) -> f64 {
        self.inner.mean
    }
    #[getter]
    fn stddev(&self) -> f64 {
        self.inner.stddev
    }
    #[getter]
    fn min(&self) -> f64 {
        self.inner.min
    }
    #[getter]
    fn max(&self) -> f64 {
        self.inner.max
    }
    #[getter]
    fn p5(&self) -> f64 {
        self.inner.p5
    }
    #[getter]
    fn p25(&self) -> f64 {
        self.inner.p25
    }
    #[getter]
    fn median(&self) -> f64 {
        self.inner.median
    }
    #[getter]
    fn p75(&self) -> f64 {
        self.inner.p75
    }
    #[getter]
    fn p95(&self) -> f64 {
        self.inner.p95
    }

    fn __repr__(&self) -> String {
        format!(
            "Distribution(mean={:.2}, stddev={:.2}, median={:.2}, p5={:.2}, p95={:.2})",
            self.inner.mean, self.inner.stddev, self.inner.median, self.inner.p5, self.inner.p95
        )
    }
}

/// Per-replica summary statistics (frozen).
#[pyclass(frozen, name = "ReplicaSummary")]
pub struct PyReplicaSummary {
    pub(crate) inner: pirx_core::ReplicaSummary,
}

#[pymethods]
impl PyReplicaSummary {
    #[getter]
    fn seed(&self) -> u64 {
        self.inner.seed
    }
    #[getter]
    fn total_cycles(&self) -> u64 {
        self.inner.total_cycles
    }
    #[getter]
    fn truncated(&self) -> bool {
        self.inner.truncated
    }
    #[getter]
    fn stall_count(&self) -> u64 {
        self.inner.stall_count
    }
    #[getter]
    fn total_stall_cycles(&self) -> u64 {
        self.inner.total_stall_cycles
    }
    #[getter]
    fn max_stall_cycles(&self) -> u64 {
        self.inner.max_stall_cycles
    }
    #[getter]
    fn injection_errors(&self) -> u64 {
        self.inner.injection_errors
    }
    #[getter]
    fn fixups_inserted(&self) -> u64 {
        self.inner.fixups_inserted
    }
    #[getter]
    fn mean_factory_utilization(&self) -> f64 {
        self.inner.mean_factory_utilization
    }
    #[getter]
    fn buffer_full_events(&self) -> u64 {
        self.inner.buffer_full_events
    }

    #[getter]
    fn magic_states_consumed(&self) -> u64 {
        self.inner.magic_states_consumed
    }

    #[getter]
    fn total_infidelity(&self) -> f64 {
        self.inner.total_infidelity
    }

    fn __repr__(&self) -> String {
        format!(
            "ReplicaSummary(seed={}, total_cycles={}, stalls={}, infidelity={:.2e})",
            self.inner.seed,
            self.inner.total_cycles,
            self.inner.stall_count,
            self.inner.total_infidelity,
        )
    }
}

/// Complete Monte Carlo simulation result (frozen).
#[pyclass(frozen, name = "MonteCarloResult")]
pub struct PyMonteCarloResult {
    pub(crate) inner: pirx_core::MonteCarloResult,
}

#[pymethods]
impl PyMonteCarloResult {
    #[getter]
    fn replicas(&self) -> Vec<PyReplicaSummary> {
        self.inner
            .replicas
            .iter()
            .map(|s| PyReplicaSummary { inner: s.clone() })
            .collect()
    }

    #[getter]
    fn total_cycles(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.total_cycles.clone(),
        }
    }

    #[getter]
    fn stall_count(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.stall_count.clone(),
        }
    }

    #[getter]
    fn total_stall_cycles(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.total_stall_cycles.clone(),
        }
    }

    #[getter]
    fn max_stall_cycles(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.max_stall_cycles.clone(),
        }
    }

    #[getter]
    fn injection_errors(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.injection_errors.clone(),
        }
    }

    #[getter]
    fn fixups_inserted(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.fixups_inserted.clone(),
        }
    }

    #[getter]
    fn mean_factory_utilization(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.mean_factory_utilization.clone(),
        }
    }

    #[getter]
    fn buffer_full_events(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.buffer_full_events.clone(),
        }
    }

    #[getter]
    fn magic_states_consumed(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.magic_states_consumed.clone(),
        }
    }

    #[getter]
    fn total_infidelity(&self) -> PyDistribution {
        PyDistribution {
            inner: self.inner.total_infidelity.clone(),
        }
    }

    #[getter]
    fn truncated_count(&self) -> u32 {
        self.inner.truncated_count
    }

    #[getter]
    fn replica_count(&self) -> u32 {
        self.inner.config.replicas
    }

    #[getter]
    fn base_seed(&self) -> u64 {
        self.inner.config.base_seed
    }

    fn __repr__(&self) -> String {
        format!(
            "MonteCarloResult(replicas={}, total_cycles=Distribution(mean={:.0}, p95={:.0}))",
            self.inner.config.replicas, self.inner.total_cycles.mean, self.inner.total_cycles.p95,
        )
    }
}
