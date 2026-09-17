#!/usr/bin/env python3
"""Exhaustive selector-domain reference; does not execute adapter Rust or DFHack."""
from itertools import combinations, product
import hashlib
import json
from pathlib import Path


def overlaps(left, right):
    return bool(set(left[0]) & set(right[0])) and all(
        a is None or b is None or a == b for a, b in zip(left[1:], right[1:]))


def matches(selector, item):
    return item[0] in selector[0] and all(
        expected is None or expected == actual
        for expected, actual in zip(selector[1:], item[1:]))


def main():
    types = ("A", "B")
    values = (-1, 0, 1)
    universe = tuple(product(types, values, values, values))
    type_sets = tuple(subset for width in (1, 2) for subset in combinations(types, width))
    material_pairs = ((None, None),) + tuple(product(values, (None,) + values))
    domains = tuple((kinds, subtype, material, index)
                    for kinds, subtype, (material, index)
                    in product(type_sets, (None,) + values, material_pairs))
    concrete = [frozenset(item for item in universe if matches(selector, item)) for selector in domains]
    pairs = 0
    disjoint = 0
    for left_index, left in enumerate(domains):
        for right_index, right in enumerate(domains):
            expected = bool(concrete[left_index] & concrete[right_index])
            assert overlaps(left, right) == expected, (left, right)
            if not expected:
                disjoint += 1
                # Every possible stack contributes to at most one resource.
                assert all(int(matches(left, item)) + int(matches(right, item)) <= 1
                           for item in universe)
            pairs += 1
    print(json.dumps({"status": "passed", "domains": len(domains),
                      "selector_pairs": pairs, "disjoint_pairs": disjoint,
                      "concrete_item_shapes": len(universe), "rust_executed": False,
                      "reference_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}, sort_keys=True))


if __name__ == "__main__":
    main()
