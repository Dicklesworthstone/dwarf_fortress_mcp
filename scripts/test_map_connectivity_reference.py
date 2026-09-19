#!/usr/bin/env python3
"""Independent small-graph checks; does not compile or execute the Rust code.

Compare an iterative low-link reference with destructive BFS for every induced
4x3 cardinal grid. Endpoint-index ordering also checks bridge partition labels.
"""
from collections import deque
import json


def components(graph, allowed, removed=None, edge=None):
    left = set(allowed)
    left.discard(removed)
    result = []
    while left:
        root = min(left)
        left.remove(root)
        queue = deque([root])
        found = {root}
        while queue:
            node = queue.popleft()
            for target in graph[node]:
                if tuple(sorted((node, target))) == edge:
                    continue
                if target in left:
                    left.remove(target)
                    found.add(target)
                    queue.append(target)
        result.append(found)
    return result


def low_link(graph, allowed):
    size = len(graph)
    seen = [0] * size
    low = [0] * size
    parent = [None] * size
    subtree = [0] * size
    splits = [[] for _ in graph]
    bridge_children = []
    groups = []
    owner = {}
    timer = 0
    for root in sorted(allowed):
        if seen[root]:
            continue
        group = set()
        ordinal = len(groups)
        groups.append(group)
        timer += 1
        seen[root] = low[root] = timer
        subtree[root] = 1
        group.add(root)
        owner[root] = ordinal
        stack = [[root, 0]]
        while stack:
            node, offset = stack[-1]
            if offset < len(graph[node]):
                target = graph[node][offset]
                stack[-1][1] += 1
                if not seen[target]:
                    timer += 1
                    seen[target] = low[target] = timer
                    parent[target] = node
                    subtree[target] = 1
                    owner[target] = ordinal
                    group.add(target)
                    stack.append([target, 0])
                elif target != parent[node]:
                    low[node] = min(low[node], seen[target])
            else:
                stack.pop()
                p = parent[node]
                if p is not None:
                    subtree[p] += subtree[node]
                    low[p] = min(low[p], low[node])
                    if low[node] >= seen[p]:
                        splits[p].append(subtree[node])
                    if low[node] > seen[p]:
                        bridge_children.append(node)
    cuts = {}
    pairs = {}
    for node in sorted(allowed):
        parts = list(splits[node])
        rest = len(groups[owner[node]]) - 1 - sum(parts)
        if rest:
            parts.append(rest)
        if len(parts) > 1:
            cuts[node] = sorted(parts, reverse=True)
            pairs[node] = sum(a*b for i, a in enumerate(parts) for b in parts[i+1:])
    bridges = {}
    for node in bridge_children:
        p = parent[node]
        child = subtree[node]
        rest = len(groups[owner[node]]) - child
        bridges[tuple(sorted((node, p)))] = [child, rest] if node < p else [rest, child]
    return groups, cuts, bridges, pairs


def verify(graph, allowed):
    groups = components(graph, allowed)
    cuts = {}
    pairs = {}
    bridges = {}
    checks = 1
    for group in groups:
        for node in sorted(group):
            parts = components(graph, group, removed=node)
            if len(parts) > 1:
                cuts[node] = sorted(map(len, parts), reverse=True)
                pairs[node] = sum(len(a)*len(b) for i, a in enumerate(parts) for b in parts[i+1:])
            checks += 1
            for target in graph[node]:
                if node >= target:
                    continue
                parts = components(graph, group, edge=(node, target))
                if len(parts) > 1:
                    left = next(len(part) for part in parts if node in part)
                    bridges[(node, target)] = [left, len(group) - left]
                checks += 1
    actual = low_link(graph, allowed)
    assert actual == (groups, cuts, bridges, pairs), (graph, allowed, actual)
    return checks


def main():
    checks = 0
    for mask in range(1 << 12):
        allowed = {n for n in range(12) if mask & (1 << n)}
        graph = [[] for _ in range(12)]
        for a in allowed:
            graph[a] = [b for b in sorted(allowed)
                        if abs(a % 4 - b % 4) + abs(a // 4 - b // 4) == 1]
        checks += verify(graph, allowed)
    # Arbitrary simple graphs stress DFS-root and non-root cases independent of
    # the grid's geometry; all 1,024 undirected graphs on five vertices.
    edges = [(a, b) for a in range(5) for b in range(a+1, 5)]
    for mask in range(1 << len(edges)):
        graph = [[] for _ in range(5)]
        for i, (a, b) in enumerate(edges):
            if mask & (1 << i):
                graph[a].append(b)
                graph[b].append(a)
        checks += verify(graph, set(range(5)))
    print(json.dumps({"graphs": 5120, "component_vertex_edge_checks": checks,
        "evidence": "Independent Python reference vs destructive BFS; not Rust, MCP, DFHack or live execution"}, indent=2))


if __name__ == "__main__":
    main()
