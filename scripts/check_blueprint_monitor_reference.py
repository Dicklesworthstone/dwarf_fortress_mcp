#!/usr/bin/env python3
"""JSON Schema checks and an independent monitor-request reference, not Rust/MCP."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]


def options_valid(value, tick=100, horizon=200):
    key = value["key"]
    cadence = value.get("poll_interval_ticks")
    stability = value.get("stable_observations")
    cadence = 1 if cadence is None else cadence
    stability = 2 if stability is None else stability
    return (0 < len(key.encode("utf-8")) <= 64 and "\0" not in key
            and tick < value["deadline_tick"] <= tick + horizon
            and 1 <= cadence <= 1_000_000
            and 1 <= stability <= 64)


def main():
    schema = json.loads((ROOT / "schemas/mcp_spatial_blueprint_v1.json").read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    templates = [
        {"kind": "bedroom_cluster", "rooms_count": 20, "room_size": [3, 3]},
        {"kind": "dining_hall", "width": 3, "height": 3},
        {"kind": "workshop_hub", "bays_count": 3},
        {"kind": "stockpile_vault", "width": 3, "height": 3, "category": "stone"},
        {"kind": "defensive_moat", "min": [10, 10, 5], "max": [16, 16, 5], "drawbridge_span": 3},
    ]
    accepted = rejected = 0
    options = [None, {"key": "goal", "deadline_tick": 200},
               {"key": "g" * 64, "deadline_tick": 200, "poll_interval_ticks": 1, "stable_observations": 1},
               {"key": "goal", "deadline_tick": 200, "poll_interval_ticks": 1_000_000, "stable_observations": 64},
               {"key": "goal", "deadline_tick": 200, "poll_interval_ticks": None, "stable_observations": None}]
    mutations = [
        ("key", ""), ("key", "g" * 65), ("key", "a\0b"), ("key", 1),
        ("deadline_tick", 0), ("deadline_tick", -1), ("deadline_tick", 2**64), ("deadline_tick", "200"),
        ("deadline_tick", True), ("stable_observations", 0), ("stable_observations", 65),
        ("poll_interval_ticks", 0), ("poll_interval_ticks", 1_000_001),
        ("commit", True), ("raw_lua", "return true"), ("override_environmental_hazards", True),
    ]
    for template in templates:
        base = {"kind": "blueprint_layout", "origin": [10, 10, 5], "template": template}
        assert validator.is_valid(base)
        accepted += 1
        for opt in options:
            query = dict(base, monitor=opt)
            assert validator.is_valid(query), query
            accepted += 1
        for field, value in mutations:
            query = deepcopy(base)
            query["monitor"] = {"key": "goal", "deadline_tick": 200, field: value}
            assert not validator.is_valid(query), query
            rejected += 1
        for missing in ("key", "deadline_tick"):
            query = dict(base, monitor={"key": "goal", "deadline_tick": 200})
            del query["monitor"][missing]
            assert not validator.is_valid(query)
            rejected += 1
        for invalid in ([], 1, "yes", True):
            assert not validator.is_valid(dict(base, monitor=invalid))
            rejected += 1
    # Additional runtime relationships are intentionally outside JSON Schema.
    relations = [({"key": "goal", "deadline_tick": tick}, expected)
                 for tick, expected in ((99, False), (100, False), (101, True), (300, True), (301, False))]
    relations += [({"key": "é" * n, "deadline_tick": 200}, expected) for n, expected in ((32, True), (33, False))]
    relations += [({"key": "goal", "deadline_tick": 200, field: n}, expected)
                  for field, n, expected in (("poll_interval_ticks", 0, False),
                      ("poll_interval_ticks", 1_000_001, False), ("stable_observations", 0, False),
                      ("stable_observations", 65, False), ("stable_observations", None, True))]
    for value, expected in relations:
        assert options_valid(value) == expected
    # Largest room count produces 55 disjoint parts, not one predicate per tile.
    # This independently checks bounded handoff shape/size, not Rust serialization.
    areas = []
    for row in range(6):
        for col in range(4):
            x, y = 10 + col * 4, 10 + row * 6
            areas += [{"min": [x, y, 5], "max": [x+2, y+2, 5]},
                      {"min": [x+1, y+3, 5], "max": [x+1, y+3, 5]}]
        areas.append({"min": [9, 14 + row*6, 5], "max": [24, 14 + row*6, 5]})
    areas.append({"min": [8, 14, 5], "max": [8, 44, 5]})
    points = set()
    for area in areas:
        for y in range(area["min"][1], area["max"][1]+1):
            for x in range(area["min"][0], area["max"][0]+1):
                point = (x, y, 5)
                assert point not in points
                points.add(point)
    assert len(areas) == 55
    condition = {"op": "terrain_count", "areas": areas, "predicate": {
        "op": "field", "field": "shape", "comparison": "eq", "value": {"type": "text", "value": "floor"}},
        "comparison": "eq", "value": len(points)}
    request = {"schema": "dfmcp.query/1", "expected_anchor": {"fortress_id": str(2**64-1),
        "cursor": {"epoch": 2**64-1, "sequence": 2**64-1}, "game_tick": 2**64-1, "state_hash": "f"*64},
        "query": {"kind": "watch", "key": "g"*64, "label": "excavate 24 bedroom units", "condition": condition,
                  "deadline_tick": 2**64-1, "poll_interval_ticks": 1, "stable_observations": 2}}
    encoded = json.dumps(request, separators=(",", ":"), ensure_ascii=False).encode()
    pending = [request]
    nodes = 0
    while pending:
        value = pending.pop()
        nodes += 1
        if isinstance(value, dict):
            pending.extend(value.values())
        elif isinstance(value, list):
            pending.extend(value)
    assert nodes <= 1024 and len(encoded) <= 32768
    print(json.dumps({"evidence": "python_schema_and_monitor_request_reference_only",
                      "schema_accepted": accepted, "schema_rejected": rejected,
                      "runtime_relationship_reference_cases": len(relations),
                      "max_room_mask_parts": len(areas), "max_room_mask_tiles": len(points),
                      "reference_request_nodes": nodes, "reference_request_bytes": len(encoded),
                      "rust_executed": False, "mcp_executed": False,
                      "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}, sort_keys=True))


if __name__ == "__main__":
    main()
