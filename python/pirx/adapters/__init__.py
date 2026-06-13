"""Pirx framework adapters."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    import pirx


def from_tket(circuit: Any, *, name: str | None = None) -> pirx.ProfilerCircuit:
    """Convert a pytket Circuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[tket]
    """
    from pirx.adapters.tket import from_tket as _impl

    return _impl(circuit, name=name)


def from_qiskit(circuit: Any, *, name: str | None = None) -> pirx.ProfilerCircuit:
    """Convert a Qiskit QuantumCircuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[qiskit]
    """
    from pirx.adapters.qiskit import from_qiskit as _impl

    return _impl(circuit, name=name)


def from_qiskit_dag(dag: Any, *, name: str | None = None) -> pirx.ProfilerCircuit:
    """Convert a Qiskit DAGCircuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[qiskit]
    """
    from pirx.adapters.qiskit import from_qiskit_dag as _impl

    return _impl(dag, name=name)


def from_qualtran(
    bloq: Any, *, name: str | None = None, max_depth: int | None = None
) -> pirx.ProfilerCircuit:
    """Convert a Qualtran Bloq to a Pirx ProfilerCircuit.

    Requires: pip install pirx[qualtran]
    """
    from pirx.adapters.qualtran import from_qualtran as _impl

    return _impl(bloq, name=name, max_depth=max_depth)
