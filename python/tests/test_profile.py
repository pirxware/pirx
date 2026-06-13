from __future__ import annotations

import json

import pirx


def test_profile_single_clifford(single_clifford_circuit, cultivation_hw):
    prof = pirx.profile(single_clifford_circuit, cultivation_hw)
    assert prof.total_cycles > 0
    assert isinstance(prof.total_cycles, int)


def test_profile_t_gate_chain(t_gate_chain_circuit, single_factory_hw):
    prof = pirx.profile(t_gate_chain_circuit, single_factory_hw, seed=7)
    assert len(prof.stall_events) > 0, "cold start with 1 factory must produce stalls"
    for s in prof.stall_events:
        assert s.wait_cycles > 0


def test_profile_deterministic(t_gate_chain_circuit, cultivation_hw):
    p1 = pirx.profile(t_gate_chain_circuit, cultivation_hw, seed=42)
    p2 = pirx.profile(t_gate_chain_circuit, cultivation_hw, seed=42)
    assert p1.total_cycles == p2.total_cycles
    assert p1.injection_errors == p2.injection_errors
    assert p1.fixups_inserted == p2.fixups_inserted


def test_profile_json_roundtrip(t_gate_chain_circuit, cultivation_hw):
    prof = pirx.profile(t_gate_chain_circuit, cultivation_hw, seed=1)
    json_str = prof.to_json()
    parsed = json.loads(json_str)
    assert parsed["total_cycles"] == prof.total_cycles
    assert parsed["resolution"] == prof.resolution
    assert len(parsed["factory_utilization"]) == len(prof.factory_utilization)


def test_profile_resolution(t_gate_chain_circuit, cultivation_hw):
    p5 = pirx.profile(t_gate_chain_circuit, cultivation_hw, seed=1, resolution=5)
    p20 = pirx.profile(t_gate_chain_circuit, cultivation_hw, seed=1, resolution=20)
    assert len(p5.factory_utilization) != len(p20.factory_utilization)
    assert p5.total_cycles == p20.total_cycles


def test_profile_max_cycles(t_gate_chain_circuit, cultivation_hw):
    prof = pirx.profile(t_gate_chain_circuit, cultivation_hw, seed=1, max_cycles=5)
    assert prof.total_cycles <= 5
