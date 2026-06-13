use pirx_ir::{
    ValidatedCircuit,
    circuit::{CircuitMetadata, Dependency, OpKind, ProfilerCircuit},
};
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PyList, PyTuple, PyType},
};

use crate::{ParseError, ValidationError};

#[pyclass(name = "ProfilerCircuit", frozen)]
pub struct PyProfilerCircuit {
    pub(crate) inner: ValidatedCircuit,
}

/// Convert a Python object to a serde_json::Value for deserialization
/// into Rust IR types. Handles None, bool, int, float, str, list, dict, tuple.
fn py_to_value(obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    use serde_json::Value;

    if obj.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(Value::Number(i.into()));
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(Value::String(s));
    }
    if let Ok(list) = obj.cast::<PyList>() {
        let items: PyResult<Vec<Value>> = list.iter().map(|item| py_to_value(&item)).collect();
        return Ok(Value::Array(items?));
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            map.insert(key, py_to_value(&v)?);
        }
        return Ok(Value::Object(map));
    }
    if let Ok(tuple) = obj.cast::<PyTuple>() {
        let items: PyResult<Vec<Value>> = tuple.iter().map(|item| py_to_value(&item)).collect();
        return Ok(Value::Array(items?));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "cannot convert {} to JSON",
        obj.get_type().qualname()?
    )))
}

#[pymethods]
impl PyProfilerCircuit {
    #[getter]
    fn qubit_count(&self) -> u32 {
        self.inner.qubit_count
    }

    #[getter]
    fn op_count(&self) -> usize {
        self.inner.ops.len()
    }

    #[getter]
    fn t_count(&self) -> u64 {
        self.inner.metadata.t_count
    }

    #[getter]
    fn clifford_count(&self) -> u64 {
        self.inner.metadata.clifford_count
    }

    #[getter]
    fn rotation_count(&self) -> u64 {
        self.inner.metadata.rotation_count
    }

    #[getter]
    fn depth(&self) -> u64 {
        self.inner.metadata.depth
    }

    #[getter]
    fn name(&self) -> &str {
        &self.inner.metadata.name
    }

    #[getter]
    fn source_framework(&self) -> &str {
        &self.inner.metadata.source_framework
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(&*self.inner).map_err(|e| ParseError::new_err(e.to_string()))
    }

    fn save_json(&self, path: &str) -> PyResult<()> {
        let json = serde_json::to_string_pretty(&*self.inner)
            .map_err(|e| ParseError::new_err(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "ProfilerCircuit(name='{}', ops={}, qubits={}, t_count={})",
            self.inner.metadata.name,
            self.inner.ops.len(),
            self.inner.qubit_count,
            self.inner.metadata.t_count,
        )
    }

    #[classmethod]
    #[pyo3(signature = (ops, deps, qubit_count, metadata, qubit_positions=None, hooks=None))]
    fn from_adapter_data(
        _cls: &Bound<'_, PyType>,
        ops: &Bound<'_, PyAny>,
        deps: Vec<(u64, u64)>,
        qubit_count: u32,
        metadata: &Bound<'_, PyAny>,
        qubit_positions: Option<&Bound<'_, PyAny>>,
        hooks: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let ops_value = py_to_value(ops)?;
        let parsed_ops: Vec<pirx_ir::circuit::Operation> = serde_json::from_value(ops_value)
            .map_err(|e| ParseError::new_err(format!("invalid ops: {e}")))?;

        let meta_value = py_to_value(metadata)?;
        let parsed_meta: CircuitMetadata = serde_json::from_value(meta_value)
            .map_err(|e| ParseError::new_err(format!("invalid metadata: {e}")))?;

        let parsed_positions = qubit_positions
            .map(|p| -> PyResult<Vec<pirx_ir::circuit::GridPosition>> {
                let val = py_to_value(p)?;
                serde_json::from_value(val)
                    .map_err(|e| ParseError::new_err(format!("invalid qubit_positions: {e}")))
            })
            .transpose()?;

        let parsed_hooks = hooks
            .map(|h| -> PyResult<Vec<pirx_ir::circuit::MeasurementHook>> {
                let val = py_to_value(h)?;
                serde_json::from_value(val)
                    .map_err(|e| ParseError::new_err(format!("invalid hooks: {e}")))
            })
            .transpose()?
            .unwrap_or_default();

        let parsed_deps: Vec<Dependency> = deps
            .into_iter()
            .map(|(from, to)| Dependency { from, to })
            .collect();

        // Recompute gate counts from actual ops to catch adapter mistakes.
        let mut t_count = 0u64;
        let mut clifford_count = 0u64;
        let mut rotation_count = 0u64;
        for op in &parsed_ops {
            match op.kind {
                OpKind::TGate => t_count += 1,
                OpKind::Clifford => clifford_count += 1,
                OpKind::Rotation { .. } => rotation_count += 1,
                OpKind::Measurement { .. } => {}
            }
        }

        let circuit_meta = CircuitMetadata {
            name: parsed_meta.name,
            source_framework: parsed_meta.source_framework,
            t_count,
            clifford_count,
            rotation_count,
            depth: parsed_meta.depth,
        };

        let circuit = ProfilerCircuit {
            ops: parsed_ops,
            deps: parsed_deps,
            qubit_count,
            qubit_positions: parsed_positions,
            hooks: parsed_hooks,
            metadata: circuit_meta,
        };

        let validated = pirx_ir::validate::validate(circuit)
            .map_err(|e| ValidationError::new_err(e.to_string()))?;
        Ok(Self { inner: validated })
    }
}
