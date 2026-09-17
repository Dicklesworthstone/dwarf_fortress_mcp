#!/usr/bin/env python3
"""Independent finite mask/count oracle, not execution of Rust or MCP."""
from itertools import product
import json
from pathlib import Path


def interval_truth(low, high, comparison, target):
    yes, no = {
        "eq": (low == high == target, target < low or target > high),
        "ne": (target < low or target > high, low == high == target),
        "lt": (high < target, low >= target),
        "le": (high <= target, low > target),
        "gt": (low > target, high <= target),
        "ge": (low >= target, high < target),
    }[comparison]
    return True if yes else False if no else None


def validate(areas):
    if not 1 <= len(areas) <= 64:
        return False
    total = 0
    for index, (low, high) in enumerate(areas):
        if any(not 0 <= low[i] <= high[i] < 32768 for i in range(3)):
            return False
        volume = 1
        for i in range(3):
            volume *= high[i] - low[i] + 1
        total += volume
        if total > 16384:
            return False
        if any(all(low[i] <= other_high[i] and other_low[i] <= high[i]
                   for i in range(3)) for other_low, other_high in areas[:index]):
            return False
    return True


def cells(area):
    low, high = area
    return set(product(*(range(low[i], high[i] + 1) for i in range(3))))


def main():
    comparisons = {
        "eq": lambda n, t: n == t, "ne": lambda n, t: n != t,
        "lt": lambda n, t: n < t, "le": lambda n, t: n <= t,
        "gt": lambda n, t: n > t, "ge": lambda n, t: n >= t,
    }
    count_cases = 0
    for values in product((True, False, None), repeat=3):
        low = sum(v is True for v in values)
        high = low + sum(v is None for v in values)
        # Enumerate assignments to unknown coordinates, not just the interval.
        completions = []
        for guesses in product((False, True), repeat=sum(v is None for v in values)):
            iterator = iter(guesses)
            completions.append(sum(next(iterator) if v is None else v for v in values))
        for name, compare in comparisons.items():
            for target in range(5):
                actual = [compare(n, target) for n in completions]
                expected = True if all(actual) else False if not any(actual) else None
                assert interval_truth(low, high, name, target) is expected
                count_cases += 1
    axes = [[(a, b) for a in range(n) for b in range(a, n)] for n in (3, 3, 2)]
    masks = [(tuple(a for a, _ in spans), tuple(b for _, b in spans)) for spans in product(*axes)]
    sets = [cells(mask) for mask in masks]
    mask_cases = 0
    for i, first in enumerate(masks):
        for j, second in enumerate(masks):
            assert validate([first, second]) == (not sets[i].intersection(sets[j]))
            mask_cases += 1
    boundary_cases = [
        ([], False), ([((0, 0, 0), (127, 127, 0))], True),
        ([((0, 0, 0), (128, 127, 0))], False),
        ([((0, 0, 0), (32768, 0, 0))], False),
        ([((2, 0, 0), (1, 0, 0))], False),
        ([((32767, 32767, 32767), (32767, 32767, 32767))], True),
        ([((i, 0, 0), (i, 0, 0)) for i in range(64)], True),
        ([((i, 0, 0), (i, 0, 0)) for i in range(65)], False),
    ]
    for areas, expected in boundary_cases:
        assert validate(areas) == expected
    schema = json.loads((Path(__file__).resolve().parents[1] / "schemas/mcp_watch_terrain_v1.json").read_text())
    assert schema["properties"]["op"]["const"] == "terrain_count"
    assert schema["properties"]["areas"]["maxItems"] == 64
    print(json.dumps({"evidence": "independent_python_mask_and_interval_reference_only",
                      "interval_cases": count_cases, "mask_pairs": mask_cases,
                      "mask_boundaries": len(boundary_cases), "schema_json_parsed": True,
                      "rust_executed": False, "mcp_executed": False,
                      "total": count_cases + mask_cases + len(boundary_cases)}, sort_keys=True))


if __name__ == "__main__":
    main()
