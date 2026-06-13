from __future__ import annotations

import math

import pytest

import pirx

qiskit = pytest.importorskip("qiskit")

from qiskit import QuantumCircuit  # noqa: E402
from qiskit.circuit import QuantumRegister  # noqa: E402
from qiskit.converters import circuit_to_dag  # noqa: E402

from pirx.adapters.qiskit import from_qiskit, from_qiskit_dag  # noqa: E402

# ---------------------------------------------------------------------------
# Gate classification
# ---------------------------------------------------------------------------


class TestGateClassification:
    def test_single_t_gate(self):
        qc = QuantumCircuit(1)
        qc.t(0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1
        assert circuit.clifford_count == 0
        assert circuit.op_count == 1

    def test_single_tdg(self):
        qc = QuantumCircuit(1)
        qc.tdg(0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1

    def test_single_h_gate(self):
        qc = QuantumCircuit(1)
        qc.h(0)
        circuit = from_qiskit(qc)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0
        assert circuit.op_count == 1

    def test_rz_pi_over_4_is_t_gate(self):
        qc = QuantumCircuit(1)
        qc.rz(math.pi / 4, 0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1
        assert circuit.rotation_count == 0

    def test_rz_pi_over_2_is_clifford(self):
        qc = QuantumCircuit(1)
        qc.rz(math.pi / 2, 0)
        circuit = from_qiskit(qc)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0

    def test_rz_arbitrary_is_rotation(self):
        qc = QuantumCircuit(1)
        qc.rz(0.3, 0)
        circuit = from_qiskit(qc)
        assert circuit.rotation_count == 1
        assert circuit.t_count == 0

    def test_rz_3pi_over_4_is_t_gate(self):
        qc = QuantumCircuit(1)
        qc.rz(3 * math.pi / 4, 0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1

    def test_rz_pi_is_clifford(self):
        qc = QuantumCircuit(1)
        qc.rz(math.pi, 0)
        circuit = from_qiskit(qc)
        assert circuit.clifford_count == 1

    def test_rx_pi_over_4_is_t_gate(self):
        qc = QuantumCircuit(1)
        qc.rx(math.pi / 4, 0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1

    def test_ry_pi_over_4_is_t_gate(self):
        qc = QuantumCircuit(1)
        qc.ry(math.pi / 4, 0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1

    def test_rx_pi_over_2_is_clifford(self):
        qc = QuantumCircuit(1)
        qc.rx(math.pi / 2, 0)
        circuit = from_qiskit(qc)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0

    def test_ry_arbitrary_is_rotation(self):
        qc = QuantumCircuit(1)
        qc.ry(0.3, 0)
        circuit = from_qiskit(qc)
        assert circuit.rotation_count == 1

    def test_p_gate_classified(self):
        qc = QuantumCircuit(1)
        qc.p(0.3, 0)
        circuit = from_qiskit(qc)
        assert circuit.rotation_count == 1

    def test_p_gate_pi_over_4_is_t(self):
        qc = QuantumCircuit(1)
        qc.p(math.pi / 4, 0)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1

    def test_measure_classified(self):
        qc = QuantumCircuit(1, 1)
        qc.h(0)
        qc.measure(0, 0)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 2

    def test_cx_is_clifford(self):
        qc = QuantumCircuit(2)
        qc.cx(0, 1)
        circuit = from_qiskit(qc)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0


# ---------------------------------------------------------------------------
# Dependencies
# ---------------------------------------------------------------------------


class TestDependencies:
    def test_dependency_chain(self):
        qc = QuantumCircuit(2)
        qc.h(0)  # op 0
        qc.t(0)  # op 1 — depends on 0 (same qubit)
        qc.cx(0, 1)  # op 2 — depends on 1 (qubit 0)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 3

    def test_parallel_ops_independent(self):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(1)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 2

    def test_cx_creates_deps_on_both_qubits(self):
        qc = QuantumCircuit(2)
        qc.h(0)  # op 0
        qc.h(1)  # op 1
        qc.cx(0, 1)  # op 2 — depends on 0 (qubit 0) and 1 (qubit 1)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 3

    def test_duplicate_deps_removed(self):
        qc = QuantumCircuit(2)
        qc.cx(0, 1)  # op 0
        qc.cx(0, 1)  # op 1 — two wires to op 0, but only one dep
        circuit = from_qiskit(qc)
        assert circuit.op_count == 2


# ---------------------------------------------------------------------------
# Metadata
# ---------------------------------------------------------------------------


class TestMetadata:
    def test_metadata_counts(self):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(0)
        qc.cx(0, 1)
        qc.rz(0.3, 1)
        circuit = from_qiskit(qc)
        assert circuit.t_count == 1
        assert circuit.clifford_count == 2  # H + CX
        assert circuit.rotation_count == 1

    def test_metadata_custom_name(self):
        qc = QuantumCircuit(1)
        qc.h(0)
        circuit = from_qiskit(qc, name="my-circuit")
        assert circuit.name == "my-circuit"

    def test_metadata_default_name(self):
        qc = QuantumCircuit(1)
        qc.h(0)
        circuit = from_qiskit(qc)
        # Qiskit assigns auto-names like "circuit-59"; from_qiskit passes it through.
        assert circuit.name == qc.name

    def test_metadata_source_framework(self):
        qc = QuantumCircuit(1)
        qc.h(0)
        circuit = from_qiskit(qc)
        assert circuit.source_framework == "qiskit"

    def test_depth_matches(self):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(0)
        qc.cx(0, 1)
        dag = circuit_to_dag(qc)
        circuit = from_qiskit(qc)
        json_str = circuit.to_json()
        assert f'"depth":{dag.depth()}' in json_str.replace(" ", "")


# ---------------------------------------------------------------------------
# Edge cases
# ---------------------------------------------------------------------------


class TestEdgeCases:
    def test_empty_circuit_raises(self):
        qc = QuantumCircuit(1)
        with pytest.raises(ValueError, match="circuit has no gates"):
            from_qiskit(qc)

    def test_barrier_skipped(self):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.barrier()
        qc.t(1)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 2

    def test_delay_skipped(self):
        qc = QuantumCircuit(1)
        qc.h(0)
        qc.delay(100, 0)
        qc.t(0)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 2

    def test_not_a_circuit_raises_type_error(self):
        with pytest.raises(TypeError, match=r"expected qiskit\.QuantumCircuit"):
            from_qiskit("not a circuit")

    def test_multiple_registers(self):
        q = QuantumRegister(2, "q")
        a = QuantumRegister(2, "a")
        qc = QuantumCircuit(q, a)
        qc.h(q[0])
        qc.t(q[1])
        qc.h(a[0])
        qc.t(a[1])
        circuit = from_qiskit(qc)
        assert circuit.qubit_count == 4
        assert circuit.op_count == 4

    def test_unbound_parameter_raises(self):
        from qiskit.circuit import Parameter

        theta = Parameter("theta")
        qc = QuantumCircuit(1)
        qc.rz(theta, 0)
        with pytest.raises(ValueError, match="unbound parameter"):
            from_qiskit(qc)

    def test_circuit_only_barriers_raises(self):
        qc = QuantumCircuit(1)
        qc.barrier()
        with pytest.raises(ValueError, match="circuit has no gates"):
            from_qiskit(qc)

    def test_circuit_only_measurements(self):
        qc = QuantumCircuit(1, 1)
        qc.measure(0, 0)
        circuit = from_qiskit(qc)
        assert circuit.op_count == 1


# ---------------------------------------------------------------------------
# from_qiskit_dag direct path
# ---------------------------------------------------------------------------


class TestFromDag:
    def test_from_dag_matches_from_circuit(self):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(0)
        qc.cx(0, 1)
        dag = circuit_to_dag(qc)
        circuit_a = from_qiskit(qc)
        circuit_b = from_qiskit_dag(dag)
        assert circuit_a.op_count == circuit_b.op_count
        assert circuit_a.t_count == circuit_b.t_count
        assert circuit_a.qubit_count == circuit_b.qubit_count
        assert circuit_a.clifford_count == circuit_b.clifford_count

    def test_from_dag_standalone(self):
        qc = QuantumCircuit(1)
        qc.t(0)
        dag = circuit_to_dag(qc)
        circuit = from_qiskit_dag(dag, name="dag-test")
        assert circuit.t_count == 1
        assert circuit.name == "dag-test"


# ---------------------------------------------------------------------------
# End-to-end
# ---------------------------------------------------------------------------


class TestEndToEnd:
    def test_profile_qiskit_circuit(self, single_factory_hw):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(0)
        qc.cx(0, 1)
        circuit = from_qiskit(qc)
        profile = pirx.profile(circuit, single_factory_hw)
        assert profile.total_cycles > 0

    def test_deterministic(self, single_factory_hw):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(0)
        qc.cx(0, 1)
        qc.t(1)
        circuit = from_qiskit(qc)
        p1 = pirx.profile(circuit, single_factory_hw, seed=42)
        p2 = pirx.profile(circuit, single_factory_hw, seed=42)
        assert p1.to_json() == p2.to_json()

    def test_json_roundtrip(self):
        qc = QuantumCircuit(2)
        qc.h(0)
        qc.t(0)
        qc.cx(0, 1)
        circuit = from_qiskit(qc)
        json_str = circuit.to_json()
        restored = pirx.read_json_str(json_str)
        assert restored.op_count == circuit.op_count
        assert restored.t_count == circuit.t_count
        assert restored.qubit_count == circuit.qubit_count

    def test_larger_circuit(self, single_factory_hw):
        qc = QuantumCircuit(4)
        qc.h(0)
        qc.cx(0, 1)
        qc.t(1)
        qc.cx(1, 2)
        qc.rz(0.3, 2)
        qc.h(3)
        qc.t(3)
        qc.cx(2, 3)
        circuit = from_qiskit(qc)
        assert circuit.qubit_count == 4
        assert circuit.t_count == 2
        assert circuit.rotation_count == 1
        profile = pirx.profile(circuit, single_factory_hw)
        assert profile.total_cycles > 0
