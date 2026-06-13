from __future__ import annotations

import json

import pirx


def test_trace_basic(single_t_circuit, cultivation_hw):
    t = pirx.trace(single_t_circuit, cultivation_hw)
    assert t.event_count > 0
    assert t.total_cycles > 0
    assert isinstance(t.seed, int)
    assert isinstance(t.schema_version, str)


def test_trace_deterministic(t_gate_chain_circuit, cultivation_hw):
    t1 = pirx.trace(t_gate_chain_circuit, cultivation_hw, seed=99)
    t2 = pirx.trace(t_gate_chain_circuit, cultivation_hw, seed=99)
    assert t1.event_count == t2.event_count
    assert t1.total_cycles == t2.total_cycles


def test_trace_json_roundtrip(single_t_circuit, cultivation_hw):
    t = pirx.trace(single_t_circuit, cultivation_hw, seed=1)
    json_str = t.to_json()
    parsed = json.loads(json_str)
    assert parsed["total_cycles"] == t.total_cycles
    assert parsed["seed"] == t.seed
    assert len(parsed["events"]) == t.event_count


def test_trace_max_cycles_truncated(t_gate_chain_circuit, cultivation_hw):
    t = pirx.trace(t_gate_chain_circuit, cultivation_hw, seed=1, max_cycles=5)
    assert t.truncated is True
    assert t.total_cycles <= 5


def test_trace_save_json(single_t_circuit, cultivation_hw, tmp_path):
    t = pirx.trace(single_t_circuit, cultivation_hw, seed=1)
    path = str(tmp_path / "trace.json")
    t.save_json(path)
    with open(path) as f:
        parsed = json.load(f)
    assert parsed["total_cycles"] == t.total_cycles


def test_trace_repr(single_t_circuit, cultivation_hw):
    t = pirx.trace(single_t_circuit, cultivation_hw)
    r = repr(t)
    assert "Trace" in r
    assert str(t.total_cycles) in r
