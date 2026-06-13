from __future__ import annotations

from pathlib import Path

import pytest

import pirx

MODELS_DIR = Path(__file__).parent.parent.parent / "models"


def test_from_toml_cultivation():
    hw = pirx.HardwareModel.from_toml(str(MODELS_DIR / "surface_code_d17_cultivation.toml"))
    assert hw.name == "surface_code_d17_cultivation_12fac"
    assert hw.code_distance == 17
    assert hw.factory_type == "cultivation"
    assert hw.factory_count == 12
    assert hw.buffer_capacity == 8


def test_from_toml_distillation():
    hw = pirx.HardwareModel.from_toml(str(MODELS_DIR / "surface_code_d17_distillation.toml"))
    assert hw.factory_type == "distillation"
    assert hw.factory_count == 4


def test_from_toml_manhattan():
    hw = pirx.HardwareModel.from_toml(
        str(MODELS_DIR / "surface_code_d17_cultivation_manhattan.toml")
    )
    assert hw.factory_type == "cultivation"
    assert hw.code_distance == 17


def test_from_toml_invalid_path():
    with pytest.raises(OSError, match="No such file"):
        pirx.HardwareModel.from_toml("/nonexistent/model.toml")


def test_from_toml_invalid_toml():
    with pytest.raises(pirx.HardwareModelError):
        pirx.HardwareModel.from_toml_str("not valid toml {{{")


def test_from_toml_even_distance():
    toml = """
[meta]
name = "bad"
description = ""

[qec]
code_type = "surface_code"
code_distance = 4
physical_error_rate = 0.001

[timing]
cycle_time_us = 1.0

[factory]
type = "cultivation"
count = 1
lambda_raw = 0.002
fault_distance = 3

[injection]

[routing]
model = "scalar"

[buffer]
capacity = 4
"""
    with pytest.raises(pirx.HardwareModelError, match="distance"):
        pirx.HardwareModel.from_toml_str(toml)


def test_hw_properties(cultivation_hw):
    assert isinstance(cultivation_hw.name, str)
    assert isinstance(cultivation_hw.code_distance, int)
    assert isinstance(cultivation_hw.factory_count, int)
    assert isinstance(cultivation_hw.factory_type, str)
    assert isinstance(cultivation_hw.buffer_capacity, int)
    assert cultivation_hw.code_distance > 0
    assert cultivation_hw.factory_count > 0
    assert cultivation_hw.buffer_capacity > 0


def test_hw_repr(cultivation_hw):
    r = repr(cultivation_hw)
    assert "HardwareModel" in r
    assert "cultivation" in r
