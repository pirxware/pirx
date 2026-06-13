use pirx_core::{ExecutionProfile, StallRecord};
use pyo3::{prelude::*, types::PyList};

use crate::ParseError;

#[pyclass(name = "StallRecord", frozen)]
pub struct PyStallRecord {
    #[pyo3(get)]
    pub(crate) cycle: u64,
    #[pyo3(get)]
    pub(crate) gate_id: u64,
    #[pyo3(get)]
    pub(crate) wait_cycles: u64,
}

#[pymethods]
impl PyStallRecord {
    fn __repr__(&self) -> String {
        format!(
            "StallRecord(cycle={}, gate_id={}, wait_cycles={})",
            self.cycle, self.gate_id, self.wait_cycles,
        )
    }
}

#[pyclass(name = "ExecutionProfile", frozen)]
pub struct PyExecutionProfile {
    pub(crate) inner: ExecutionProfile,
}

#[pymethods]
impl PyExecutionProfile {
    #[getter]
    fn total_cycles(&self) -> u64 {
        self.inner.total_cycles
    }

    #[getter]
    fn resolution(&self) -> u64 {
        self.inner.resolution
    }

    #[getter]
    fn factory_utilization<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.inner.factory_utilization)
    }

    #[getter]
    fn buffer_occupancy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.inner.buffer_occupancy)
    }

    #[getter]
    fn stall_events(&self) -> Vec<PyStallRecord> {
        self.inner
            .stall_events
            .iter()
            .map(|s| PyStallRecord {
                cycle: s.cycle,
                gate_id: s.gate_id,
                wait_cycles: s.wait_cycles,
            })
            .collect()
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
    fn critical_path_extension(&self) -> u64 {
        self.inner.critical_path_extension
    }

    #[getter]
    fn p_logical(&self) -> f64 {
        self.inner.p_logical
    }

    #[getter]
    fn magic_states_consumed(&self) -> u64 {
        self.inner.magic_states_consumed
    }

    #[getter]
    fn total_infidelity(&self) -> f64 {
        self.inner.total_infidelity
    }

    #[getter]
    fn cumulative_magic_states<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.inner.cumulative_magic_states)
    }

    #[getter]
    fn cumulative_infidelity<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.inner.cumulative_infidelity)
    }

    #[getter]
    fn magic_states_per_bucket<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, &self.inner.magic_states_per_bucket)
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(&self.inner).map_err(|e| ParseError::new_err(e.to_string()))
    }

    fn save_json(&self, path: &str) -> PyResult<()> {
        let json = serde_json::to_string_pretty(&self.inner)
            .map_err(|e| ParseError::new_err(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "ExecutionProfile(cycles={}, stalls={}, fixups={}, infidelity={:.2e})",
            self.inner.total_cycles,
            self.inner.stall_events.len(),
            self.inner.fixups_inserted,
            self.inner.total_infidelity,
        )
    }
}

impl From<&StallRecord> for PyStallRecord {
    fn from(s: &StallRecord) -> Self {
        Self {
            cycle: s.cycle,
            gate_id: s.gate_id,
            wait_cycles: s.wait_cycles,
        }
    }
}
