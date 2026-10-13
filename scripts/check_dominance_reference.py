#!/usr/bin/env python3
"""Independent algorithm-design check; this does NOT execute or qualify Rust.

Compare the reverse-postorder algorithm implemented in graph_query/dominance.rs
with a vertex-removal reachability oracle. No dependency or network is required.
"""
from __future__ import annotations

from collections import deque
from itertools import product
import json


def reachable(adjacency: list[list[int]], root: int, removed: int | None = None) -> set[int]:
    if root == removed:
        return set()
    seen = {root}
    queue = deque([root])
    while queue:
        node = queue.popleft()
        for target in adjacency[node]:
            if target != removed and target not in seen:
                seen.add(target)
                queue.append(target)
    return seen


def analyze(adjacency: list[list[int]], root: int) -> tuple[dict[int, int | None], dict[int, int]]:
    """Design mirror, deliberately separate from the removal-based oracle."""
    size = len(adjacency)
    incoming: list[list[int]] = [[] for _ in range(size)]
    for node, targets in enumerate(adjacency):
        for target in targets:
            incoming[target].append(node)
    seen = {root}
    stack = [(root, 0)]
    order: list[int] = []
    while stack:
        node, offset = stack[-1]
        if offset < len(adjacency[node]):
            stack[-1] = node, offset + 1
            target = adjacency[node][offset]
            if target not in seen:
                seen.add(target)
                stack.append((target, 0))
        else:
            order.append(node)
            stack.pop()
    order.reverse()
    rank = {node: position for position, node in enumerate(order)}
    parents = {root: root}
    for _ in range(size + 1):
        changed = False
        for node in order[1:]:
            parent = None
            for predecessor in incoming[node]:
                if predecessor not in parents:
                    continue
                if parent is None:
                    parent = predecessor
                else:
                    left, right = parent, predecessor
                    while left != right:
                        if rank[left] > rank[right]:
                            left = parents[left]
                        else:
                            right = parents[right]
                    parent = left
            assert parent is not None and rank[parent] < rank[node]
            if parents.get(node) != parent:
                parents[node] = parent
                changed = True
        if not changed:
            break
    else:
        raise AssertionError("fixed point failed to converge")
    counts = {node: 1 for node in order}
    for node in reversed(order[1:]):
        counts[parents[node]] += counts[node]
    parents[root] = None
    return parents, {node: count - 1 for node, count in counts.items()}


def verify(adjacency: list[list[int]], root: int) -> None:
    parents, impacts = analyze(adjacency, root)
    baseline = reachable(adjacency, root)
    assert set(parents) == baseline
    mandatory = {
        target: {node for node in baseline if target not in reachable(adjacency, root, node)}
        for target in baseline
    }
    for node in baseline:
        chain: set[int] = set()
        cursor: int | None = node
        while cursor is not None:
            assert cursor not in chain
            chain.add(cursor)
            cursor = parents[cursor]
        assert chain == mandatory[node], (adjacency, root, node, chain, mandatory[node])
        expected = len(baseline - reachable(adjacency, root, node) - {node})
        assert impacts[node] == expected, (adjacency, root, node, impacts[node], expected)


def main() -> None:
    cases = 0
    # Every simple directed graph through four vertices, every possible root.
    for size in range(1, 5):
        pairs = [(a, b) for a in range(size) for b in range(size) if a != b]
        for mask in range(1 << len(pairs)):
            adjacency: list[list[int]] = [[] for _ in range(size)]
            for bit, (a, b) in enumerate(pairs):
                if mask & (1 << bit):
                    adjacency[a].append(b)
            for root in range(size):
                verify(adjacency, root)
                cases += 1
    # Parallel edges and self loops do not change vertex dominance.
    multigraph_cases = 0
    for multiplicities in product(range(3), repeat=9):
        adjacency = [[] for _ in range(3)]
        for offset, multiplicity in enumerate(multiplicities):
            adjacency[offset // 3].extend([offset % 3] * multiplicity)
        for root in range(3):
            verify(adjacency, root)
            multigraph_cases += 1
    print(json.dumps({
        "evidence_class": "python_algorithm_design_only",
        "rust_executed": False,
        "simple_directed_root_cases": cases,
        "multigraph_root_cases": multigraph_cases,
        "status": "passed",
    }, sort_keys=True))


if __name__ == "__main__":
    main()
