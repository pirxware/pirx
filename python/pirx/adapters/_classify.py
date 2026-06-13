"""Shared gate classification — semantic definition of OpKind for parameterized gates.

This is the Python equivalent of pirx_ir::circuit::classify_rz_angle in Rust.
Single source of truth for all Python adapters. The tolerance (1e-10) and
classification rule must match the Rust implementation exactly.
"""

from __future__ import annotations

import math
from typing import Any


def classify_rz_angle(angle_rad: float) -> dict[str, Any] | str:
    """Classify a rotation angle (in radians) into OpKind.

    Odd multiples of pi/4 -> TGate, even multiples -> Clifford,
    everything else -> Rotation.
    """
    if not math.isfinite(angle_rad):
        return {"Rotation": {"angle": angle_rad}}
    k = angle_rad / (math.pi / 4)
    k_rounded = round(k)
    if abs(k - k_rounded) < 1e-10:
        if int(k_rounded) % 2 != 0:
            return "TGate"
        return "Clifford"
    return {"Rotation": {"angle": angle_rad}}
