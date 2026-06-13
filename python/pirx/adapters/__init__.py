"""Pirx framework adapters."""


def from_tket(circuit, *, name=None):
    """Convert a pytket Circuit to a Pirx ProfilerCircuit.

    Requires: pip install pirx[tket]
    """
    from pirx.adapters.tket import from_tket as _impl

    return _impl(circuit, name=name)
