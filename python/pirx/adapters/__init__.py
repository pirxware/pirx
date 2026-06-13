"""Pirx framework adapters."""


def from_tket(circuit, *, name=None):
    """Convert a pytket Circuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[tket]
    """
    from pirx.adapters.tket import from_tket as _impl

    return _impl(circuit, name=name)


def from_qiskit(circuit, *, name=None):
    """Convert a Qiskit QuantumCircuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[qiskit]
    """
    from pirx.adapters.qiskit import from_qiskit as _impl

    return _impl(circuit, name=name)


def from_qiskit_dag(dag, *, name=None):
    """Convert a Qiskit DAGCircuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[qiskit]
    """
    from pirx.adapters.qiskit import from_qiskit_dag as _impl

    return _impl(dag, name=name)
