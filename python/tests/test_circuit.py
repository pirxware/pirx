from __future__ import annotations

from pathlib import Path

import pytest

import pirx

FIXTURES_DIR = Path(__file__).parent.parent.parent / "crates" / "pirx-cli" / "tests" / "fixtures"


def test_read_json_valid():
    path = str(FIXTURES_DIR / "t_gate_chain_3.pirx.json")
    circuit = pirx.read_json(path)
    assert circuit.op_count == 3
    assert circuit.t_count == 3
    assert circuit.qubit_count == 1
    assert circuit.name == "t-gate-chain"


def test_read_json_invalid_path():
    with pytest.raises(OSError, match="No such file"):
        pirx.read_json("/nonexistent/path.json")


def test_read_json_invalid_json():
    import tempfile

    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
        f.write("not valid json {{{")
        f.flush()
        with pytest.raises(pirx.ParseError):
            pirx.read_json(f.name)


def test_read_json_str_valid():
    import json

    circuit_dict = {
        "ops": [
            {"id": 0, "kind": "TGate", "qubits": [0]},
            {"id": 1, "kind": "TGate", "qubits": [0]},
        ],
        "deps": [{"from": 0, "to": 1}],
        "qubit_count": 1,
        "qubit_positions": None,
        "hooks": [],
        "metadata": {
            "name": "test",
            "source_framework": "test",
            "t_count": 2,
            "clifford_count": 0,
            "rotation_count": 0,
            "depth": 2,
        },
    }
    circuit = pirx.read_json_str(json.dumps(circuit_dict))
    assert circuit.op_count == 2
    assert circuit.t_count == 2


def test_read_json_cyclic_dag():
    import json

    circuit_dict = {
        "ops": [
            {"id": 0, "kind": "Clifford", "qubits": [0]},
            {"id": 1, "kind": "Clifford", "qubits": [0]},
        ],
        "deps": [{"from": 0, "to": 1}, {"from": 1, "to": 0}],
        "qubit_count": 1,
        "qubit_positions": None,
        "hooks": [],
        "metadata": {
            "name": "cyclic",
            "source_framework": "test",
            "t_count": 0,
            "clifford_count": 2,
            "rotation_count": 0,
            "depth": 1,
        },
    }
    with pytest.raises(pirx.ValidationError, match="cycle"):
        pirx.read_json_str(json.dumps(circuit_dict))


def test_from_adapter_data_valid():
    circuit = pirx.ProfilerCircuit.from_adapter_data(
        ops=[
            {"id": 0, "kind": "TGate", "qubits": [0]},
            {"id": 1, "kind": "Clifford", "qubits": [0, 1]},
            {"id": 2, "kind": {"Rotation": {"angle": 0.7854}}, "qubits": [2]},
        ],
        deps=[(0, 1)],
        qubit_count=3,
        metadata={
            "name": "adapter-test",
            "source_framework": "test",
            "t_count": 1,
            "clifford_count": 1,
            "rotation_count": 1,
            "depth": 2,
        },
    )
    assert circuit.op_count == 3
    assert circuit.t_count == 1
    assert circuit.clifford_count == 1
    assert circuit.rotation_count == 1
    assert circuit.qubit_count == 3


def test_from_adapter_data_invalid_qubit():
    with pytest.raises(pirx.ValidationError, match="qubit"):
        pirx.ProfilerCircuit.from_adapter_data(
            ops=[{"id": 0, "kind": "TGate", "qubits": [99]}],
            deps=[],
            qubit_count=1,
            metadata={
                "name": "bad",
                "source_framework": "test",
                "t_count": 1,
                "clifford_count": 0,
                "rotation_count": 0,
                "depth": 1,
            },
        )


def test_circuit_properties(single_t_circuit):
    assert single_t_circuit.qubit_count == 1
    assert single_t_circuit.t_count == 1
    assert single_t_circuit.clifford_count == 0
    assert single_t_circuit.op_count == 1
    assert single_t_circuit.depth == 1
    assert single_t_circuit.name == "single-t"
    assert single_t_circuit.source_framework == "test"


def test_circuit_to_json(single_t_circuit):
    import json

    json_str = single_t_circuit.to_json()
    parsed = json.loads(json_str)
    assert parsed["qubit_count"] == 1
    assert len(parsed["ops"]) == 1
    assert parsed["metadata"]["name"] == "single-t"


def test_circuit_save_json(single_t_circuit, tmp_path):
    path = str(tmp_path / "circuit.json")
    single_t_circuit.save_json(path)
    reloaded = pirx.read_json(path)
    assert reloaded.op_count == single_t_circuit.op_count
    assert reloaded.qubit_count == single_t_circuit.qubit_count
