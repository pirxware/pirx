from __future__ import annotations

from pathlib import Path

import pytest

import pirx

MODELS_DIR = Path(__file__).parent.parent.parent / "models"

SINGLE_FACTORY_TOML = """
[meta]
name = "test-single-factory"
description = ""

[qec]
code_type = "surface_code"
code_distance = 7
physical_error_rate = 0.001

[timing]
cycle_time_us = 1.0

[factory]
type = "cultivation"
count = 1
lambda_raw = 0.002
fault_distance = 3

[injection]
error_probability = 0.5
fixup_cost_cycles = 1

[routing]
model = "scalar"

[buffer]
capacity = 4
"""


@pytest.fixture
def cultivation_hw():
    return pirx.HardwareModel.from_toml(str(MODELS_DIR / "surface_code_d17_cultivation.toml"))


@pytest.fixture
def single_factory_hw():
    return pirx.HardwareModel.from_toml_str(SINGLE_FACTORY_TOML)


@pytest.fixture
def distillation_hw():
    return pirx.HardwareModel.from_toml(str(MODELS_DIR / "surface_code_d17_distillation.toml"))


@pytest.fixture
def single_t_circuit():
    return pirx.ProfilerCircuit.from_adapter_data(
        ops=[{"id": 0, "kind": "TGate", "qubits": [0]}],
        deps=[],
        qubit_count=1,
        metadata={
            "name": "single-t",
            "source_framework": "test",
            "t_count": 1,
            "clifford_count": 0,
            "rotation_count": 0,
            "depth": 1,
        },
    )


@pytest.fixture
def single_clifford_circuit():
    return pirx.ProfilerCircuit.from_adapter_data(
        ops=[{"id": 0, "kind": "Clifford", "qubits": [0]}],
        deps=[],
        qubit_count=1,
        metadata={
            "name": "single-clifford",
            "source_framework": "test",
            "t_count": 0,
            "clifford_count": 1,
            "rotation_count": 0,
            "depth": 1,
        },
    )


@pytest.fixture
def t_gate_chain_circuit():
    n = 5
    ops = [{"id": i, "kind": "TGate", "qubits": [0]} for i in range(n)]
    deps = [(i, i + 1) for i in range(n - 1)]
    return pirx.ProfilerCircuit.from_adapter_data(
        ops=ops,
        deps=deps,
        qubit_count=1,
        metadata={
            "name": "t-gate-chain",
            "source_framework": "test",
            "t_count": n,
            "clifford_count": 0,
            "rotation_count": 0,
            "depth": n,
        },
    )
