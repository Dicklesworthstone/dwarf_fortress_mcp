#!/usr/bin/env python3
"""Independent JSON sizing, not execution of Rust, archives, or MCP handlers."""
from __future__ import annotations
import hashlib
import itertools
import json
from pathlib import Path


def encoded(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def main() -> None:
    count = 0
    largest_growth = -10**9
    for fortress, epoch, sequence, tick, record, entity, generation, kind in itertools.product(
        [1, 2**64 - 1], [0, 2**64 - 1], [0, 2**64 - 1], [0, 2**64 - 1],
        [1, 4096], [9, 2**31 + 1], [1, 2**32 - 1], ["inspect", "traverse"],
    ):
        anchor = {"fortress_id": str(fortress), "epoch": epoch, "sequence": sequence,
                  "game_tick": tick, "state_hash": "a" * 64}
        query = ({"kind": kind, "entity_id": str(entity), "generation": generation,
                  "fields": ["worker_assigned", "worker_entity", "worker_is_strict_citizen", "position"]}
                 if kind == "inspect" else {"kind": kind, "roots": [str(entity)],
                     "edge_kinds": ["uses", "contained_in"], "max_depth": 4})
        original = {"schema": "dfmcp.query/1", "expected_anchor": anchor, "query": query}
        replacement = {"schema": "dfmcp.query/1", "query": {"kind": "historical_query",
                       "record": record, "record_digest": "b" * 64, "query": query}}
        growth = len(encoded(replacement)) - len(encoded(original))
        assert growth <= 0, (original, replacement, growth)
        largest_growth = max(largest_growth, growth)
        count += 1
    print(json.dumps({"status": "passed", "cases": count,
        "maximum_wrapper_growth_bytes": largest_growth,
        "scope": "independent compact JSON sizes for assignment/relationship wrapper templates",
        "rust_executed": False, "mcp_executed": False, "archive_replay_executed": False,
        "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}, indent=2))


if __name__ == "__main__":
    main()
