"""Adapter converting a pytket Circuit to a Pirx ProfilerCircuit."""

from __future__ import annotations

import math
import warnings
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from pytket.circuit import Circuit, Command, OpType, Qubit

try:
    from pytket.circuit import Circuit as _Circuit
    from pytket.circuit import OpType as _OpType
except ImportError as e:
    raise ImportError(
        "pytket is required for the tket adapter. Install with: pip install pirx[tket]"
    ) from e

import pirx

_T_GATE_OPS: frozenset[OpType] = frozenset(
    {
        _OpType.T,
        _OpType.Tdg,
    }
)

_ROTATION_OPS: frozenset[OpType] = frozenset(
    {
        _OpType.Rz,
        _OpType.Rx,
        _OpType.Ry,
    }
)

_MEASURE_OPS: frozenset[OpType] = frozenset(
    {
        _OpType.Measure,
        _OpType.Reset,
    }
)

_SKIP_OPS: frozenset[OpType] = frozenset(
    {
        _OpType.Barrier,
        _OpType.Phase,
        _OpType.noop,
    }
)


def _classify_rz_angle(angle_half_turns: float) -> dict[str, Any] | str:
    """Classify an Rz rotation angle (in half-turns, pytket convention) into OpKind.

    Converts to radians (multiply by pi), then applies the same logic as
    pirx_ir::classify_rz_angle: odd multiples of pi/4 -> TGate, even
    multiples -> Clifford, everything else -> Rotation.
    """
    angle_rad = angle_half_turns * math.pi
    k = angle_rad / (math.pi / 4)
    k_rounded = round(k)
    if abs(k - k_rounded) < 1e-10:
        if int(k_rounded) % 2 != 0:
            return "TGate"
        return "Clifford"
    return {"Rotation": {"angle": angle_rad}}


def _classify_op(cmd: Command) -> dict[str, Any] | str:
    """Classify a pytket Command into a pirx OpKind value."""
    op_type = cmd.op.type

    if op_type in _T_GATE_OPS:
        return "TGate"

    if op_type in _ROTATION_OPS:
        params = cmd.op.params
        if not params:
            return "Clifford"
        angle_half_turns = params[0]
        if isinstance(angle_half_turns, float | int):
            return _classify_rz_angle(float(angle_half_turns))
        warnings.warn(
            f"Symbolic parameter in {op_type.name}({angle_half_turns}) — "
            f"classifying as Rotation with angle 0.0. "
            f"Resolve symbolic parameters before calling from_tket().",
            stacklevel=3,
        )
        return {"Rotation": {"angle": 0.0}}

    if op_type in _MEASURE_OPS:
        return {"Measurement": {"hook": None}}

    return "Clifford"


def _build_qubit_map(circuit: Circuit) -> dict[Qubit, int]:
    """Map pytket Qubit objects to flat 0-based QubitId integers."""
    return {q: i for i, q in enumerate(circuit.qubits)}


def from_tket(
    circuit: Circuit,
    *,
    name: str | None = None,
) -> pirx.ProfilerCircuit:
    """Convert a pytket Circuit to a Pirx ProfilerCircuit.

    The circuit should already be compiled/decomposed to a gate set
    containing Clifford gates, T/Tdg, Rz, and measurements. Gates
    not in the FTQC gate set are classified as Clifford (conservative
    assumption — they consume no magic states).

    Parameters
    ----------
    circuit : pytket.Circuit
        A pytket Circuit. Not modified.
    name : str, optional
        Circuit name for metadata. Defaults to circuit.name or "tket_circuit".

    Returns
    -------
    pirx.ProfilerCircuit
        A validated ProfilerCircuit ready for pirx.profile().

    Raises
    ------
    pirx.ValidationError
        If the resulting circuit is structurally invalid.
    TypeError
        If circuit is not a pytket Circuit.
    ValueError
        If the circuit has no gates.
    """
    if not isinstance(circuit, _Circuit):
        raise TypeError(f"expected pytket.Circuit, got {type(circuit).__name__}")

    qubit_map = _build_qubit_map(circuit)
    commands = circuit.get_commands()

    ops: list[dict[str, Any]] = []
    deps: list[tuple[int, int]] = []
    last_op_on_qubit: dict[int, int] = {}
    op_id = 0

    for cmd in commands:
        if cmd.op.type in _SKIP_OPS:
            continue

        kind = _classify_op(cmd)
        qubit_indices = [qubit_map[q] for q in cmd.qubits]

        ops.append(
            {
                "id": op_id,
                "kind": kind,
                "qubits": qubit_indices,
            }
        )

        for q_idx in qubit_indices:
            if q_idx in last_op_on_qubit:
                deps.append((last_op_on_qubit[q_idx], op_id))
            last_op_on_qubit[q_idx] = op_id

        op_id += 1

    if not ops:
        raise ValueError("circuit has no gates")

    t_count = sum(1 for op in ops if op["kind"] == "TGate")
    rotation_count = sum(
        1 for op in ops if isinstance(op["kind"], dict) and "Rotation" in op["kind"]
    )
    measure_count = sum(
        1 for op in ops if isinstance(op["kind"], dict) and "Measurement" in op["kind"]
    )
    clifford_count = len(ops) - t_count - rotation_count - measure_count

    circuit_name = name or getattr(circuit, "name", None) or "tket_circuit"

    metadata = {
        "name": circuit_name,
        "source_framework": "tket",
        "t_count": t_count,
        "clifford_count": clifford_count,
        "rotation_count": rotation_count,
        "depth": circuit.depth(),
    }

    return pirx.ProfilerCircuit.from_adapter_data(
        ops=ops,
        deps=deps,
        qubit_count=len(qubit_map),
        metadata=metadata,
    )
