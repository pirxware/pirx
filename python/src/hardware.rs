use pirx_hw::model::{FactoryConfig, HardwareModel};
use pyo3::{prelude::*, types::PyType};

use crate::HardwareModelError;

#[pyclass(name = "HardwareModel", frozen)]
pub struct PyHardwareModel {
    pub(crate) inner: HardwareModel,
}

#[pymethods]
impl PyHardwareModel {
    #[classmethod]
    fn from_toml(_cls: &Bound<'_, PyType>, path: &str) -> PyResult<Self> {
        let contents = std::fs::read_to_string(path)?;
        let hw = pirx_hw::model::load(&contents)
            .map_err(|e| HardwareModelError::new_err(e.to_string()))?;
        Ok(Self { inner: hw })
    }

    #[classmethod]
    fn from_toml_str(_cls: &Bound<'_, PyType>, toml: &str) -> PyResult<Self> {
        let hw =
            pirx_hw::model::load(toml).map_err(|e| HardwareModelError::new_err(e.to_string()))?;
        Ok(Self { inner: hw })
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.meta.name
    }

    #[getter]
    fn code_distance(&self) -> u32 {
        self.inner.qec.code_distance
    }

    #[getter]
    fn factory_count(&self) -> u32 {
        self.inner.factory.count()
    }

    #[getter]
    fn factory_type(&self) -> &str {
        match &self.inner.factory {
            FactoryConfig::Distillation { .. } => "distillation",
            FactoryConfig::Cultivation { .. } => "cultivation",
            FactoryConfig::RzSynthesis { .. } => "rz_synthesis",
        }
    }

    #[getter]
    fn buffer_capacity(&self) -> u32 {
        self.inner.buffer.capacity
    }

    fn __repr__(&self) -> String {
        format!(
            "HardwareModel(name='{}', d={}, factories={}x{})",
            self.inner.meta.name,
            self.inner.qec.code_distance,
            self.inner.factory.count(),
            self.factory_type(),
        )
    }
}
