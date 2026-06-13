"""Adapter converting a Qualtran Bloq to a Pirx ProfilerCircuit."""

from __future__ import annotations

import logging
import math
import warnings
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from qualtran import Bloq, BloqInstance, CompositeBloq

try:
    from qualtran import Bloq as _Bloq
    from qualtran import CompositeBloq as _CompositeBloq
except ImportError as e:
    raise ImportError(
        "qualtran is required for the Qualtran adapter. Install with: pip install pirx[qualtran]"
    ) from e

import pirx
from pirx.adapters._classify import classify_rz_angle as _classify_rz_angle

logger = logging.getLogger(__name__)

_MAX_DEFAULT_DEPTH = 100

_LEAF_GATE_NAMES: frozenset[str] = frozenset(
    {
        "TGate",
        "Hadamard",
        "CNOT",
        "CSwap",
        "XGate",
        "YGate",
        "ZGate",
        "SGate",
        "Rz",
        "Rx",
        "Ry",
        "GlobalPhase",
        "Toffoli",
        "Swap",
        "TwoBitCSwap",
        "XPowGate",
        "YPowGate",
        "ZPowGate",
    }
)

_SKIP_BLOQ_NAMES: frozenset[str] = frozenset(
    {
        "Identity",
        "GlobalPhase",
        "Split",
        "Join",
        "Allocate",
        "Free",
        "Cast",
        "Partition",
    }
)

_T_GATE_NAMES: frozenset[str] = frozenset({"TGate"})

_ROTATION_NAMES: frozenset[str] = frozenset({"Rz", "Rx", "Ry", "ZPowGate", "XPowGate", "YPowGate"})


def _get_bloq_angle(bloq: Bloq) -> float | None:
    """Extract a rotation angle in radians from a Bloq, or None if not a rotation."""
    for attr in ("angle", "rad"):
        val = getattr(bloq, attr, None)
        if val is not None:
            try:
                return float(val)
            except (TypeError, ValueError):
                return None

    exponent = getattr(bloq, "exponent", None)
    if exponent is not None:
        try:
            return float(exponent) * math.pi
        except (TypeError, ValueError):
            return None

    return None


def _classify_bloq(bloq: Bloq) -> dict[str, Any] | str | None:
    """Classify a leaf Bloq into a pirx OpKind value.

    Returns None for bloqs that should be skipped.
    """
    name = type(bloq).__name__

    if name in _SKIP_BLOQ_NAMES:
        return None

    if name in _T_GATE_NAMES:
        return "TGate"

    if name in _ROTATION_NAMES:
        angle = _get_bloq_angle(bloq)
        if angle is not None:
            return _classify_rz_angle(angle)
        return {"Rotation": {"angle": 0.0}}

    return "Clifford"


class _RegisterFlattener:
    """Maps Qualtran soquet endpoints to flat contiguous QubitId integers."""

    def __init__(self) -> None:
        self._map: dict[tuple[int, str, int], int] = {}
        self._next_id: int = 0

    def get_qubit_id(self, binst: BloqInstance, reg_name: str, idx: int) -> int:
        key = (id(binst), reg_name, idx)
        if key not in self._map:
            self._map[key] = self._next_id
            self._next_id += 1
        return self._map[key]

    @property
    def qubit_count(self) -> int:
        return self._next_id


def _is_leaf(bloq: Bloq) -> bool:
    """True if this Bloq should not be decomposed further."""
    return type(bloq).__name__ in _LEAF_GATE_NAMES


def _is_skip(bloq: Bloq) -> bool:
    """True if this Bloq is an infrastructure bloq that produces no operation."""
    return type(bloq).__name__ in _SKIP_BLOQ_NAMES


def _handle_opaque_bloq(
    bloq: Bloq,
    op_id_start: int,
) -> tuple[list[dict[str, Any]], list[tuple[int, int]], int]:
    """Generate synthetic ops for a non-decomposable Bloq."""
    t_count = 0
    try:
        tc = bloq.t_complexity()
        t_count = getattr(tc, "t", 0) or 0
        t_count = int(t_count)
    except (TypeError, NotImplementedError, AttributeError):
        pass

    if t_count == 0:
        warnings.warn(
            f"Bloq '{type(bloq).__name__}' has no decomposition and no T-count — "
            f"treating as a single Clifford. Profiling result may underestimate "
            f"resource demand for this component.",
            stacklevel=4,
        )
        return (
            [{"id": op_id_start, "kind": "Clifford", "qubits": [0]}],
            [],
            op_id_start + 1,
        )

    ops: list[dict[str, Any]] = []
    deps: list[tuple[int, int]] = []
    for i in range(t_count):
        op_id = op_id_start + i
        ops.append({"id": op_id, "kind": "TGate", "qubits": [0]})
        if i > 0:
            deps.append((op_id - 1, op_id))

    return ops, deps, op_id_start + t_count


def _extract_ops(
    bloq: Bloq,
    flattener: _RegisterFlattener,
    max_depth: int,
    current_depth: int,
    next_id: int,
) -> tuple[list[dict[str, Any]], list[tuple[int, int]], int]:
    """Recursively decompose a Bloq and extract leaf operations and dependencies.

    Replaces the previous decompose/flatten/extract pipeline with a single
    recursive walk that uses only public Qualtran APIs (decompose_bloq,
    as_composite_bloq, connections, bloq_instances).
    """
    from qualtran import DecomposeNotImplementedError

    if _is_skip(bloq):
        return [], [], next_id

    if current_depth >= max_depth:
        if _is_leaf(bloq):
            kind = _classify_bloq(bloq)
            if kind is None:
                return [], [], next_id
            return [{"id": next_id, "kind": kind, "qubits": [0]}], [], next_id + 1
        return _handle_opaque_bloq(bloq, next_id)

    cbloq: CompositeBloq | None = None

    if _is_leaf(bloq):
        try:
            cbloq = bloq.as_composite_bloq()
        except (AttributeError, TypeError):
            kind = _classify_bloq(bloq)
            if kind is None:
                return [], [], next_id
            return [{"id": next_id, "kind": kind, "qubits": [0]}], [], next_id + 1
    else:
        try:
            cbloq = bloq.decompose_bloq()
        except (DecomposeNotImplementedError, NotImplementedError):
            try:
                cbloq = bloq.as_composite_bloq()
            except (AttributeError, TypeError):
                return _handle_opaque_bloq(bloq, next_id)

    if cbloq is None or not isinstance(cbloq, _CompositeBloq):
        return _handle_opaque_bloq(bloq, next_id)

    ops: list[dict[str, Any]] = []
    deps: list[tuple[int, int]] = []
    binst_to_first: dict[int, int] = {}
    binst_to_last: dict[int, int] = {}

    for binst in cbloq.bloq_instances:
        if _is_skip(binst.bloq):
            continue

        if _is_leaf(binst.bloq):
            kind = _classify_bloq(binst.bloq)
            if kind is None:
                continue

            qubits: list[int] = []
            for reg in binst.bloq.signature:
                for idx in range(reg.total_bits()):
                    qid = flattener.get_qubit_id(binst, reg.name, idx)
                    qubits.append(qid)

            seen: set[int] = set()
            unique_qubits: list[int] = []
            for q in qubits:
                if q not in seen:
                    seen.add(q)
                    unique_qubits.append(q)

            if not unique_qubits:
                warnings.warn(
                    f"Bloq '{type(binst.bloq).__name__}' produced no qubit mappings "
                    f"— assigning placeholder qubit 0.",
                    stacklevel=3,
                )
                unique_qubits = [0]

            op_id = next_id
            next_id += 1
            binst_to_first[id(binst)] = op_id
            binst_to_last[id(binst)] = op_id

            ops.append(
                {
                    "id": op_id,
                    "kind": kind,
                    "qubits": unique_qubits,
                }
            )

        else:
            first_id = next_id
            sub_ops, sub_deps, next_id = _extract_ops(
                binst.bloq, flattener, max_depth, current_depth + 1, next_id
            )
            if sub_ops:
                binst_to_first[id(binst)] = first_id
                binst_to_last[id(binst)] = next_id - 1
                ops.extend(sub_ops)
                deps.extend(sub_deps)

    for conn in cbloq.connections:
        left_binst = conn.left.binst
        right_binst = conn.right.binst

        left_id = binst_to_last.get(id(left_binst))
        right_id = binst_to_first.get(id(right_binst))
        if left_id is not None and right_id is not None and left_id != right_id:
            deps.append((left_id, right_id))

    return ops, deps, next_id


def from_qualtran(
    bloq: Bloq,
    *,
    name: str | None = None,
    max_depth: int | None = None,
) -> pirx.ProfilerCircuit:
    """Convert a Qualtran Bloq to a Pirx ProfilerCircuit.

    Recursively decomposes the Bloq to leaf-level gates (TGate, Rz,
    CNOT, etc.) and extracts the dependency DAG from Qualtran's
    connection graph.

    The Bloq should be fully parameterized (no symbolic values).
    Symbolic Bloqs cannot be decomposed to concrete gates.

    Parameters
    ----------
    bloq : qualtran.Bloq
        A Qualtran Bloq. Not modified.
    name : str, optional
        Circuit name for metadata. Defaults to type(bloq).__name__.
    max_depth : int, optional
        Maximum decomposition depth. None = decompose to leaves
        (with a safety limit of 100). Useful for very deep hierarchies
        where full decomposition is too slow or unnecessary.

    Returns
    -------
    pirx.ProfilerCircuit
        A validated ProfilerCircuit ready for pirx.profile().

    Raises
    ------
    pirx.ValidationError
        If the resulting circuit is structurally invalid.
    TypeError
        If bloq is not a Qualtran Bloq.
    ValueError
        If the Bloq cannot be decomposed or produces zero operations.
    """
    if not isinstance(bloq, _Bloq):
        raise TypeError(f"expected qualtran.Bloq, got {type(bloq).__name__}")

    effective_max_depth = max_depth if max_depth is not None else _MAX_DEFAULT_DEPTH

    flattener = _RegisterFlattener()
    ops, deps, _ = _extract_ops(bloq, flattener, effective_max_depth, 0, 0)

    if not ops:
        raise ValueError(
            f"decomposition of {type(bloq).__name__} produced zero operations "
            f"after filtering infrastructure bloqs"
        )

    deps = list(dict.fromkeys(deps))

    t_count = sum(1 for op in ops if op["kind"] == "TGate")
    rotation_count = sum(
        1 for op in ops if isinstance(op["kind"], dict) and "Rotation" in op["kind"]
    )
    measure_count = sum(
        1 for op in ops if isinstance(op["kind"], dict) and "Measurement" in op["kind"]
    )
    clifford_count = len(ops) - t_count - rotation_count - measure_count

    try:
        tc = bloq.t_complexity()
        expected_t = getattr(tc, "t", None)
        if expected_t is not None:
            expected_t = int(expected_t)
            if expected_t > 0 and abs(t_count - expected_t) > expected_t * 0.01:
                warnings.warn(
                    f"T-count mismatch: decomposition found {t_count}, "
                    f"t_complexity() reports {expected_t}. "
                    f"This may indicate incomplete decomposition.",
                    stacklevel=2,
                )
    except (TypeError, NotImplementedError, AttributeError):
        pass

    if len(ops) > 1_000_000:
        logger.info("Qualtran adapter: extracted %d operations", len(ops))

    circuit_name = name or type(bloq).__name__

    qubit_count = flattener.qubit_count
    if qubit_count == 0:
        qubit_count = 1

    metadata = {
        "name": circuit_name,
        "source_framework": "qualtran",
        "t_count": t_count,
        "clifford_count": clifford_count,
        "rotation_count": rotation_count,
        "depth": 0,
    }

    return pirx.ProfilerCircuit.from_adapter_data(
        ops=ops,
        deps=deps,
        qubit_count=qubit_count,
        metadata=metadata,
    )
