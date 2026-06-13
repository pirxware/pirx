from __future__ import annotations

import pytest

import pirx

pytket = pytest.importorskip("pytket")

from pytket import Circuit  # noqa: E402

from pirx.adapters.tket import from_tket  # noqa: E402

# ---------------------------------------------------------------------------
# Gate classification
# ---------------------------------------------------------------------------


class TestGateClassification:
    def test_single_t_gate(self):
        c = Circuit(1)
        c.T(0)
        circuit = from_tket(c)
        assert circuit.t_count == 1
        assert circuit.clifford_count == 0
        assert circuit.op_count == 1

    def test_single_tdg(self):
        c = Circuit(1)
        c.Tdg(0)
        circuit = from_tket(c)
        assert circuit.t_count == 1

    def test_single_clifford(self):
        c = Circuit(1)
        c.H(0)
        circuit = from_tket(c)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0
        assert circuit.op_count == 1

    def test_rz_pi_over_4_is_t_gate(self):
        c = Circuit(1)
        c.Rz(0.25, 0)  # 0.25 half-turns = pi/4 radians
        circuit = from_tket(c)
        assert circuit.t_count == 1
        assert circuit.rotation_count == 0

    def test_rz_pi_over_2_is_clifford(self):
        c = Circuit(1)
        c.Rz(0.5, 0)  # 0.5 half-turns = pi/2 radians
        circuit = from_tket(c)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0

    def test_rz_arbitrary_is_rotation(self):
        c = Circuit(1)
        c.Rz(0.3, 0)  # 0.3 half-turns = arbitrary angle
        circuit = from_tket(c)
        assert circuit.rotation_count == 1
        assert circuit.t_count == 0

    def test_rz_3pi_over_4_is_t_gate(self):
        c = Circuit(1)
        c.Rz(0.75, 0)  # 0.75 half-turns = 3*pi/4 radians (odd multiple of pi/4)
        circuit = from_tket(c)
        assert circuit.t_count == 1

    def test_rz_pi_is_clifford(self):
        c = Circuit(1)
        c.Rz(1.0, 0)  # 1.0 half-turns = pi radians
        circuit = from_tket(c)
        assert circuit.clifford_count == 1

    def test_rx_pi_over_4_is_t_gate(self):
        c = Circuit(1)
        c.Rx(0.25, 0)  # 0.25 half-turns = pi/4 radians
        circuit = from_tket(c)
        assert circuit.t_count == 1

    def test_ry_pi_over_4_is_t_gate(self):
        c = Circuit(1)
        c.Ry(0.25, 0)
        circuit = from_tket(c)
        assert circuit.t_count == 1

    def test_rx_pi_over_2_is_clifford(self):
        c = Circuit(1)
        c.Rx(0.5, 0)  # 0.5 half-turns = pi/2 radians
        circuit = from_tket(c)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0

    def test_ry_arbitrary_is_rotation(self):
        c = Circuit(1)
        c.Ry(0.3, 0)
        circuit = from_tket(c)
        assert circuit.rotation_count == 1

    def test_measure_classified(self):
        c = Circuit(1, 1)
        c.H(0)
        c.Measure(0, 0)
        circuit = from_tket(c)
        assert circuit.op_count == 2

    def test_multi_controlled_gate_is_clifford(self):
        c = Circuit(3)
        c.CCX(0, 1, 2)
        circuit = from_tket(c)
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0


# ---------------------------------------------------------------------------
# Dependencies
# ---------------------------------------------------------------------------


class TestDependencies:
    def test_dependency_chain(self):
        c = Circuit(2)
        c.H(0)  # op 0
        c.T(0)  # op 1 — depends on 0 (same qubit)
        c.CX(0, 1)  # op 2 — depends on 1 (qubit 0)
        circuit = from_tket(c)
        assert circuit.op_count == 3

    def test_parallel_ops_no_deps(self):
        c = Circuit(2)
        c.H(0)
        c.T(1)
        circuit = from_tket(c)
        assert circuit.op_count == 2

    def test_two_qubit_gate_creates_deps_on_both(self):
        c = Circuit(2)
        c.H(0)  # op 0
        c.H(1)  # op 1
        c.CX(0, 1)  # op 2 — depends on 0 (qubit 0) and 1 (qubit 1)
        circuit = from_tket(c)
        assert circuit.op_count == 3


# ---------------------------------------------------------------------------
# Metadata
# ---------------------------------------------------------------------------


class TestMetadata:
    def test_metadata_counts(self):
        c = Circuit(2)
        c.H(0)
        c.T(0)
        c.CX(0, 1)
        c.Rz(0.3, 1)
        circuit = from_tket(c)
        assert circuit.t_count == 1
        assert circuit.clifford_count == 2  # H + CX
        assert circuit.rotation_count == 1

    def test_metadata_custom_name(self):
        c = Circuit(1)
        c.H(0)
        circuit = from_tket(c, name="my-circuit")
        assert circuit.name == "my-circuit"

    def test_metadata_default_name(self):
        c = Circuit(1)
        c.H(0)
        circuit = from_tket(c)
        assert circuit.name == "tket_circuit"

    def test_metadata_source_framework(self):
        c = Circuit(1)
        c.H(0)
        circuit = from_tket(c)
        assert circuit.source_framework == "tket"


# ---------------------------------------------------------------------------
# Edge cases
# ---------------------------------------------------------------------------


class TestEdgeCases:
    def test_empty_circuit_raises(self):
        c = Circuit(1)
        with pytest.raises(ValueError, match="circuit has no gates"):
            from_tket(c)

    def test_barrier_skipped(self):
        c = Circuit(2)
        c.H(0)
        c.add_barrier([0, 1])
        c.T(1)
        circuit = from_tket(c)
        assert circuit.op_count == 2  # barrier not counted

    def test_not_a_circuit_raises_type_error(self):
        with pytest.raises(TypeError, match=r"expected pytket\.Circuit"):
            from_tket("not a circuit")

    def test_qubit_mapping_contiguous(self):
        c = Circuit()
        q0 = c.add_q_register("q", 2)
        q1 = c.add_q_register("a", 1)
        c.H(q0[0])
        c.T(q0[1])
        c.H(q1[0])
        circuit = from_tket(c)
        assert circuit.qubit_count == 3
        assert circuit.op_count == 3

    def test_circuit_only_measurements(self):
        c = Circuit(1, 1)
        c.Measure(0, 0)
        circuit = from_tket(c)
        assert circuit.op_count == 1

    def test_symbolic_parameter_raises(self):
        from sympy import Symbol

        c = Circuit(1)
        c.Rz(Symbol("theta"), 0)
        with pytest.raises(ValueError, match="symbolic parameter"):
            from_tket(c)


# ---------------------------------------------------------------------------
# classify_rz_angle equivalence (Python copy vs Rust pirx-ir)
# ---------------------------------------------------------------------------


class TestClassifyRzAngleEquivalence:
    """Verify that the Python _classify_rz_angle produces the same OpKind
    as the Rust classify_rz_angle (via from_adapter_data recomputation)."""

    def test_equivalence_via_tket_roundtrip(self):
        angles = [0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, -0.25, -0.75, 0.3, 0.123, 3.0]
        for ht in angles:
            c = Circuit(1)
            c.Rz(ht, 0)
            circuit = from_tket(c)
            rust_t = circuit.t_count
            rust_rot = circuit.rotation_count
            rust_cliff = circuit.clifford_count

            if ht == 0.0:
                assert rust_cliff == 1, f"angle {ht}: expected Clifford"
            elif rust_t == 1:
                assert rust_rot == 0, f"angle {ht}: TGate but rotation_count!=0"
            elif rust_rot == 1:
                assert rust_t == 0, f"angle {ht}: Rotation but t_count!=0"
            else:
                assert rust_cliff == 1, f"angle {ht}: expected exactly one kind"


# ---------------------------------------------------------------------------
# End-to-end
# ---------------------------------------------------------------------------


class TestEndToEnd:
    def test_profile_tket_circuit(self, single_factory_hw):
        c = Circuit(2)
        c.H(0)
        c.T(0)
        c.CX(0, 1)
        circuit = from_tket(c)
        profile = pirx.profile(circuit, single_factory_hw)
        assert profile.total_cycles > 0

    def test_deterministic_roundtrip(self, single_factory_hw):
        c = Circuit(2)
        c.H(0)
        c.T(0)
        c.CX(0, 1)
        c.T(1)
        circuit = from_tket(c)
        p1 = pirx.profile(circuit, single_factory_hw, seed=42)
        p2 = pirx.profile(circuit, single_factory_hw, seed=42)
        assert p1.to_json() == p2.to_json()

    def test_json_roundtrip(self):
        c = Circuit(2)
        c.H(0)
        c.T(0)
        c.CX(0, 1)
        circuit = from_tket(c)
        json_str = circuit.to_json()
        restored = pirx.read_json_str(json_str)
        assert restored.op_count == circuit.op_count
        assert restored.t_count == circuit.t_count
        assert restored.qubit_count == circuit.qubit_count

    def test_larger_circuit(self, single_factory_hw):
        c = Circuit(4)
        c.H(0).CX(0, 1).T(1).CX(1, 2).Rz(0.3, 2)
        c.H(3).T(3).CX(2, 3)
        circuit = from_tket(c)
        assert circuit.qubit_count == 4
        assert circuit.t_count == 2
        assert circuit.rotation_count == 1
        profile = pirx.profile(circuit, single_factory_hw)
        assert profile.total_cycles > 0
