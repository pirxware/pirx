from __future__ import annotations

import pytest

import pirx

qualtran = pytest.importorskip("qualtran")

from qualtran import DecomposeNotImplementedError, Signature  # noqa: E402
from qualtran._infra.gate_with_registers import GateWithRegisters  # noqa: E402
from qualtran.bloqs.basic_gates import CNOT, Hadamard, Rx, Ry, Rz, Swap, TGate, XGate  # noqa: E402

from pirx.adapters.qualtran import from_qualtran  # noqa: E402

# ---------------------------------------------------------------------------
# Gate classification
# ---------------------------------------------------------------------------


class TestGateClassification:
    def test_single_t_gate(self):
        circuit = from_qualtran(TGate())
        assert circuit.t_count == 1
        assert circuit.op_count == 1

    def test_t_gate_adjoint(self):
        circuit = from_qualtran(TGate(is_adjoint=True))
        assert circuit.t_count == 1

    def test_single_hadamard(self):
        circuit = from_qualtran(Hadamard())
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0
        assert circuit.op_count == 1

    def test_single_cnot(self):
        circuit = from_qualtran(CNOT())
        assert circuit.clifford_count == 1
        assert circuit.qubit_count >= 2

    def test_rz_arbitrary(self):
        circuit = from_qualtran(Rz(angle=0.3))
        assert circuit.rotation_count == 1
        assert circuit.t_count == 0

    def test_x_gate_is_clifford(self):
        circuit = from_qualtran(XGate())
        assert circuit.clifford_count == 1
        assert circuit.t_count == 0

    def test_rx_pi_over_4_is_tgate(self):
        import math

        circuit = from_qualtran(Rx(angle=math.pi / 4))
        assert circuit.t_count == 1
        assert circuit.rotation_count == 0

    def test_ry_pi_over_4_is_tgate(self):
        import math

        circuit = from_qualtran(Ry(angle=math.pi / 4))
        assert circuit.t_count == 1
        assert circuit.rotation_count == 0

    def test_rx_pi_over_2_is_clifford(self):
        import math

        circuit = from_qualtran(Rx(angle=math.pi / 2))
        assert circuit.clifford_count >= 1
        assert circuit.t_count == 0

    def test_ry_arbitrary_is_rotation(self):
        circuit = from_qualtran(Ry(angle=0.3))
        assert circuit.rotation_count == 1
        assert circuit.t_count == 0

    def test_rz_pi_over_4_is_tgate(self):
        import math

        circuit = from_qualtran(Rz(angle=math.pi / 4))
        assert circuit.t_count == 1
        assert circuit.rotation_count == 0


# ---------------------------------------------------------------------------
# Dependencies
# ---------------------------------------------------------------------------


class TestDependencies:
    def test_leaf_bloq_no_deps(self):
        circuit = from_qualtran(TGate())
        assert circuit.op_count == 1

    def test_swap_is_clifford(self):
        circuit = from_qualtran(Swap(bitsize=1))
        assert circuit.op_count >= 1


# ---------------------------------------------------------------------------
# Metadata
# ---------------------------------------------------------------------------


class TestMetadata:
    def test_metadata_custom_name(self):
        circuit = from_qualtran(TGate(), name="my-bloq")
        assert circuit.name == "my-bloq"

    def test_metadata_default_name(self):
        circuit = from_qualtran(TGate())
        assert circuit.name == "TGate"

    def test_metadata_source_framework(self):
        circuit = from_qualtran(TGate())
        assert circuit.source_framework == "qualtran"


# ---------------------------------------------------------------------------
# Edge cases
# ---------------------------------------------------------------------------


class TestEdgeCases:
    def test_not_a_bloq_raises_type_error(self):
        with pytest.raises(TypeError, match=r"expected qualtran\.Bloq"):
            from_qualtran("not a bloq")

    def test_max_depth_zero(self):
        circuit = from_qualtran(TGate(), max_depth=0)
        assert circuit.op_count >= 1

    def test_max_depth_one(self):
        circuit = from_qualtran(Swap(bitsize=1), max_depth=1)
        assert circuit.op_count >= 1

    def test_t_count_cross_validation_warning(self):
        class _MismatchBloq(GateWithRegisters):
            """Reports t_complexity=100 but decomposes to a single Hadamard (0 T gates)."""

            @property
            def signature(self) -> Signature:
                return Signature.build(q=1)

            def decompose_bloq(self):
                return Hadamard().as_composite_bloq()

            def t_complexity(self):
                from qualtran.cirq_interop.t_complexity_protocol import TComplexity

                return TComplexity(t=100)

        with pytest.warns(match="T-count mismatch"):
            circuit = from_qualtran(_MismatchBloq())
        assert circuit.t_count == 0


# ---------------------------------------------------------------------------
# Opaque bloq handling
# ---------------------------------------------------------------------------


class TestOpaqueBloqs:
    def test_opaque_without_t_complexity_warns(self):
        class _OpaqueBloq(GateWithRegisters):
            @property
            def signature(self) -> Signature:
                return Signature.build(q=1)

            def decompose_bloq(self):
                raise DecomposeNotImplementedError(self)

        with pytest.warns(match="no decomposition and no T-count"):
            circuit = from_qualtran(_OpaqueBloq())
        assert circuit.op_count >= 1
        assert circuit.clifford_count >= 1


# ---------------------------------------------------------------------------
# End-to-end
# ---------------------------------------------------------------------------


class TestEndToEnd:
    def test_profile_qualtran_bloq(self, single_factory_hw):
        circuit = from_qualtran(TGate())
        profile = pirx.profile(circuit, single_factory_hw)
        assert profile.total_cycles > 0

    def test_deterministic(self, single_factory_hw):
        circuit = from_qualtran(CNOT())
        p1 = pirx.profile(circuit, single_factory_hw, seed=42)
        p2 = pirx.profile(circuit, single_factory_hw, seed=42)
        assert p1.to_json() == p2.to_json()

    def test_json_roundtrip(self):
        circuit = from_qualtran(TGate())
        json_str = circuit.to_json()
        restored = pirx.read_json_str(json_str)
        assert restored.op_count == circuit.op_count
        assert restored.t_count == circuit.t_count
        assert restored.qubit_count == circuit.qubit_count
