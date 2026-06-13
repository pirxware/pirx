mod circuit;
mod hardware;
mod pymontecarlo;
mod pyprofile;
mod pytrace;

pub(crate) use circuit::PyProfilerCircuit;
pub(crate) use hardware::PyHardwareModel;
pub(crate) use pymontecarlo::{PyDistribution, PyMonteCarloResult, PyReplicaSummary};
use pyo3::prelude::*;
pub(crate) use pyprofile::{PyExecutionProfile, PyStallRecord};
pub(crate) use pytrace::PyTrace;

pyo3::create_exception!(pirx, ValidationError, pyo3::exceptions::PyValueError);
pyo3::create_exception!(pirx, HardwareModelError, pyo3::exceptions::PyValueError);
pyo3::create_exception!(pirx, EngineError, pyo3::exceptions::PyRuntimeError);
pyo3::create_exception!(pirx, ParseError, pyo3::exceptions::PyValueError);

#[pyfunction]
#[pyo3(signature = (circuit, hw, *, seed=42, max_cycles=None, resolution=10))]
fn profile(
    py: Python<'_>,
    circuit: &PyProfilerCircuit,
    hw: &PyHardwareModel,
    seed: u64,
    max_cycles: Option<u64>,
    resolution: u64,
) -> PyResult<PyExecutionProfile> {
    let config = pirx_core::EngineConfig { seed, max_cycles };
    let circuit_ref = &circuit.inner;
    let hw_ref = &hw.inner;
    #[allow(clippy::cast_possible_truncation)]
    let factory_count = hw_ref.factory.count() as u16;

    let result = py.detach(|| -> Result<pirx_core::ExecutionProfile, String> {
        let engine =
            pirx_core::Engine::new(circuit_ref, hw_ref, config).map_err(|e| e.to_string())?;
        let trace = engine.run();
        Ok(pirx_core::ProfileAnalyzer::analyze(
            &trace,
            factory_count,
            resolution,
        ))
    });

    match result {
        Ok(prof) => Ok(PyExecutionProfile { inner: prof }),
        Err(msg) => Err(EngineError::new_err(msg)),
    }
}

#[pyfunction]
#[pyo3(signature = (circuit, hw, *, seed=42, max_cycles=None))]
fn trace(
    py: Python<'_>,
    circuit: &PyProfilerCircuit,
    hw: &PyHardwareModel,
    seed: u64,
    max_cycles: Option<u64>,
) -> PyResult<PyTrace> {
    let config = pirx_core::EngineConfig { seed, max_cycles };
    let circuit_ref = &circuit.inner;
    let hw_ref = &hw.inner;

    let result = py.detach(|| -> Result<pirx_core::Trace, String> {
        let engine =
            pirx_core::Engine::new(circuit_ref, hw_ref, config).map_err(|e| e.to_string())?;
        Ok(engine.run())
    });

    match result {
        Ok(t) => Ok(PyTrace { inner: t }),
        Err(msg) => Err(EngineError::new_err(msg)),
    }
}

#[pyfunction]
#[pyo3(signature = (circuit, hw, *, replicas=100, seed=42, max_cycles=None, threads=None))]
fn monte_carlo(
    py: Python<'_>,
    circuit: &PyProfilerCircuit,
    hw: &PyHardwareModel,
    replicas: u32,
    seed: u64,
    max_cycles: Option<u64>,
    threads: Option<usize>,
) -> PyResult<PyMonteCarloResult> {
    let circuit_ref = &circuit.inner;
    let hw_ref = &hw.inner;

    let result = py.detach(|| -> Result<pirx_core::MonteCarloResult, String> {
        let mc_config = pirx_core::MonteCarloConfig {
            replicas,
            base_seed: seed,
            max_cycles,
            threads,
        };
        pirx_core::run_monte_carlo(circuit_ref, hw_ref, mc_config).map_err(|e| e.to_string())
    });

    match result {
        Ok(r) => Ok(PyMonteCarloResult { inner: r }),
        Err(msg) => Err(EngineError::new_err(msg)),
    }
}

#[pyfunction]
fn read_json(path: &str) -> PyResult<PyProfilerCircuit> {
    let contents = std::fs::read_to_string(path)?;
    let circuit: pirx_ir::circuit::ProfilerCircuit =
        serde_json::from_str(&contents).map_err(|e| ParseError::new_err(e.to_string()))?;
    let validated = pirx_ir::validate::validate(circuit)
        .map_err(|e| ValidationError::new_err(e.to_string()))?;
    Ok(PyProfilerCircuit { inner: validated })
}

#[pyfunction]
fn read_json_str(json: &str) -> PyResult<PyProfilerCircuit> {
    let circuit: pirx_ir::circuit::ProfilerCircuit =
        serde_json::from_str(json).map_err(|e| ParseError::new_err(e.to_string()))?;
    let validated = pirx_ir::validate::validate(circuit)
        .map_err(|e| ValidationError::new_err(e.to_string()))?;
    Ok(PyProfilerCircuit { inner: validated })
}

#[pymodule]
fn _pirx(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyProfilerCircuit>()?;
    m.add_class::<PyHardwareModel>()?;
    m.add_class::<PyExecutionProfile>()?;
    m.add_class::<PyTrace>()?;
    m.add_class::<PyStallRecord>()?;
    m.add_class::<PyMonteCarloResult>()?;
    m.add_class::<PyReplicaSummary>()?;
    m.add_class::<PyDistribution>()?;
    m.add_function(wrap_pyfunction!(profile, m)?)?;
    m.add_function(wrap_pyfunction!(trace, m)?)?;
    m.add_function(wrap_pyfunction!(monte_carlo, m)?)?;
    m.add_function(wrap_pyfunction!(read_json, m)?)?;
    m.add_function(wrap_pyfunction!(read_json_str, m)?)?;
    m.add("ValidationError", m.py().get_type::<ValidationError>())?;
    m.add(
        "HardwareModelError",
        m.py().get_type::<HardwareModelError>(),
    )?;
    m.add("EngineError", m.py().get_type::<EngineError>())?;
    m.add("ParseError", m.py().get_type::<ParseError>())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
