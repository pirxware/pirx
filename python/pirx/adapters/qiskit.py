"""Adapter converting a Qiskit QuantumCircuit to a Pirx ProfilerCircuit."""

from __future__ import annotations

import math
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from qiskit.circuit import QuantumCircuit
    from qiskit.dagcircuit import DAGCircuit

try:
    from qiskit.circuit import QuantumCircuit as _QuantumCircuit
    from qiskit.converters import circuit_to_dag
except ImportError as e:
    raise ImportError(
        "qiskit is required for the Qiskit adapter. Install with: pip install pirx[qiskit]"
    ) from e

import pirx

_T_GATE_NAMES: frozenset[str] = frozenset({"t", "tdg"})

_MEASURE_NAMES: frozenset[str] = frozenset({"measure", "reset"})

_ROTATION_NAMES: frozenset[str] = frozenset({"rz", "rx", "ry", "p"})

_SKIP_NAMES: frozenset[str] = frozenset({"barrier", "delay", "id", "snapshot"})


def _classify_rz_angle(angle_rad: float) -> dict[str, Any] | str:
    """Classify an Rz rotation angle (in radians, Qiskit convention) into OpKind.

    Odd multiples of pi/4 -> TGate, even multiples -> Clifford,
    everything else -> Rotation.
    """
    k = angle_rad / (math.pi / 4)
    k_rounded = round(k)
    if abs(k - k_rounded) < 1e-10:
        if int(k_rounded) % 2 != 0:
            return "TGate"
        return "Clifford"
    return {"Rotation": {"angle": angle_rad}}


def _classify_op(node) -> dict[str, Any] | str | None:
    """Classify a DAGOpNode into a pirx OpKind value.

    Returns None for operations that should be skipped (barrier, delay, id).
    """
    op_name = node.op.name.lower()

    if op_name in _SKIP_NAMES:
        return None

    if op_name in _T_GATE_NAMES:
        return "TGate"

    if op_name in _MEASURE_NAMES:
        return {"Measurement": {"hook": None}}

    if op_name in _ROTATION_NAMES:
        params = node.op.params
        if params:
            param = params[0]
            try:
                angle = float(param)
            except (TypeError, ValueError) as err:
                raise ValueError(
                    f"circuit contains unbound parameter '{param}' in gate '{node.op.name}' "
                    f"— transpile/bind parameters before calling from_qiskit()"
                ) from err
            return _classify_rz_angle(angle)
        return "Clifford"

    # Classical-only ops (no quantum operands) are skipped.
    if not node.qargs:
        return None

    # Everything else: Clifford (conservative).
    return "Clifford"


def _build_qubit_map(dag) -> dict:
    """Map Qiskit Qubit objects to flat 0-based QubitId integers."""
    return {q: i for i, q in enumerate(dag.qubits)}


def _extract_from_dag(dag, qubit_map: dict) -> tuple[list[dict[str, Any]], list[tuple[int, int]]]:
    """Extract operations and dependencies from a Qiskit DAGCircuit.

    Uses dag.topological_op_nodes() for operations and dag.edges() for
    dependencies. Classical-only edges are ignored.
    """
    ops: list[dict[str, Any]] = []
    node_to_id: dict[int, int] = {}
    next_id = 0

    for node in dag.topological_op_nodes():
        kind = _classify_op(node)
        if kind is None:
            continue

        op_id = next_id
        next_id += 1
        node_to_id[node._node_id] = op_id

        qubit_indices = [qubit_map[q] for q in node.qargs]
        ops.append(
            {
                "id": op_id,
                "kind": kind,
                "qubits": qubit_indices,
            }
        )

    deps: list[tuple[int, int]] = []
    for src, dst, _ in dag.edges():
        src_id = node_to_id.get(getattr(src, "_node_id", None))
        dst_id = node_to_id.get(getattr(dst, "_node_id", None))
        if src_id is not None and dst_id is not None:
            deps.append((src_id, dst_id))

    # Deduplicate: multi-qubit gates sharing wires produce duplicate edges.
    deps = list(dict.fromkeys(deps))

    return ops, deps


def from_qiskit_dag(
    dag: DAGCircuit,
    *,
    name: str | None = None,
) -> pirx.ProfilerCircuit:
    """Convert a Qiskit DAGCircuit directly to a Pirx ProfilerCircuit.

    Use this when you already have a DAGCircuit (e.g., inside a transpiler
    pass) to avoid the QuantumCircuit -> DAGCircuit conversion overhead.

    Parameters
    ----------
    dag : qiskit.dagcircuit.DAGCircuit
        A Qiskit DAGCircuit. Not modified.
    name : str, optional
        Circuit name for metadata. Defaults to dag.name or "qiskit_circuit".

    Returns
    -------
    pirx.ProfilerCircuit
        A validated ProfilerCircuit ready for pirx.profile().

    Raises
    ------
    pirx.ValidationError
        If the resulting circuit is structurally invalid.
    ValueError
        If the circuit has no gates after filtering barriers/delays,
        or contains unbound parameters.
    """
    qubit_map = _build_qubit_map(dag)
    ops, deps = _extract_from_dag(dag, qubit_map)

    if not ops:
        raise ValueError("circuit has no gates after filtering barriers/delays")

    t_count = sum(1 for op in ops if op["kind"] == "TGate")
    rotation_count = sum(
        1 for op in ops if isinstance(op["kind"], dict) and "Rotation" in op["kind"]
    )
    measure_count = sum(
        1 for op in ops if isinstance(op["kind"], dict) and "Measurement" in op["kind"]
    )
    clifford_count = len(ops) - t_count - rotation_count - measure_count

    circuit_name = name or getattr(dag, "name", None) or "qiskit_circuit"

    metadata = {
        "name": circuit_name,
        "source_framework": "qiskit",
        "t_count": t_count,
        "clifford_count": clifford_count,
        "rotation_count": rotation_count,
        "depth": dag.depth(),
    }

    return pirx.ProfilerCircuit.from_adapter_data(
        ops=ops,
        deps=deps,
        qubit_count=len(qubit_map),
        metadata=metadata,
    )


def from_qiskit(
    circuit: QuantumCircuit,
    *,
    name: str | None = None,
) -> pirx.ProfilerCircuit:
    """Convert a Qiskit QuantumCircuit to a Pirx ProfilerCircuit.

    Internally converts to DAGCircuit for dependency extraction.
    The circuit should already be transpiled to a gate set containing
    Clifford gates, T/Tdg, Rz, and measurements.

    Parameters
    ----------
    circuit : qiskit.QuantumCircuit
        A Qiskit QuantumCircuit. Not modified.
    name : str, optional
        Circuit name for metadata. Defaults to circuit.name or "qiskit_circuit".

    Returns
    -------
    pirx.ProfilerCircuit
        A validated ProfilerCircuit ready for pirx.profile().

    Raises
    ------
    pirx.ValidationError
        If the resulting circuit is structurally invalid.
    TypeError
        If circuit is not a QuantumCircuit.
    ValueError
        If the circuit has no gates after filtering barriers/delays,
        or contains unbound parameters.
    """
    if not isinstance(circuit, _QuantumCircuit):
        raise TypeError(f"expected qiskit.QuantumCircuit, got {type(circuit).__name__}")
    dag = circuit_to_dag(circuit)
    return from_qiskit_dag(dag, name=name or circuit.name)
