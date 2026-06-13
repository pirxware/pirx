use pirx_core::Trace;
use pyo3::prelude::*;

use crate::ParseError;

#[pyclass(name = "Trace", frozen)]
pub struct PyTrace {
    pub(crate) inner: Trace,
}

#[pymethods]
impl PyTrace {
    #[getter]
    fn total_cycles(&self) -> u64 {
        self.inner.total_cycles
    }

    #[getter]
    fn seed(&self) -> u64 {
        self.inner.seed
    }

    #[getter]
    fn truncated(&self) -> bool {
        self.inner.truncated
    }

    #[getter]
    fn event_count(&self) -> usize {
        self.inner.events.len()
    }

    #[getter]
    fn schema_version(&self) -> &str {
        &self.inner.schema_version
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
            "Trace(cycles={}, events={}, seed={}, truncated={})",
            self.inner.total_cycles,
            self.inner.events.len(),
            self.inner.seed,
            self.inner.truncated,
        )
    }
}
