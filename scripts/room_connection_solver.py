"""Exact bounded new-tile minimization for an already admitted room capture.

Internal pure solver for room_connections. Every original floor component is a
terminal; extra dry-floor claims, route authority and partial plans are absent.
Existing floors cost zero, candidate corridor cells cost one, blocked cells are
not vertices. Subset dynamic programming joins components at a shared vertex;
multi-source Dijkstra propagates each subset. One caller-owned guard accounts
for the entire attempt, including failed greedy work and reconstruction.

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10).
"""
from __future__ import annotations

from array import array
from collections import deque
from dataclasses import dataclass
import heapq
from typing import Callable

Point = tuple[int, int, int]
MAX_COMPONENTS = 9  # Eight internally connected areas and an external entry.
MAX_CELLS = 1024
MAX_TABLE_BYTES = 8 * 1024 * 1024
TIE_POLICY = 'component-coordinate-mask-split-ascending-strict-cost-tile-heap/1'


class ComponentLimit(ValueError):
    """The solver did not establish feasibility within its component bound."""


class NoConnection(ValueError):
    """No complete connection fits the admitted region and tile allowance."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _neighbors(p: Point) -> tuple[Point, ...]:
    x, y, z = p
    return (x - 1, y, z), (x, y - 1, z), (x, y + 1, z), (x + 1, y, z)


@dataclass(frozen=True)
class Minimum:
    added: tuple[Point, ...]
    representatives: tuple[Point, ...]
    subset_count: int
    cost: int
    table_bytes: int


def minimum_tiles(floors: set[Point], admitted: set[Point], root: Point,
                  allowance: int, work: Callable[[], None]) -> Minimum:
    """Return an exact whole connection, or raise without a usable prefix.

    The enclosing connector has already validated the coordinate/domain contract.
    Recheck structural limits before any exponential allocation here. There is no
    independent timeout, budget renewal, effect, or caught caller cancellation.
    Equal costs retain the first deterministic derivation. A subset merge pays
    its shared vertex only once; a move pays only the destination vertex.
    """
    work()
    _require(type(root) is tuple and len(root) == 3
             and all(type(n) is int and 1 <= n <= 32766 for n in root),
             'minimum connection root needs three exact coordinates')
    _require(type(floors) is set and type(admitted) is set
             and 1 <= len(floors) <= 512 and len(admitted) <= MAX_CELLS
             and floors <= admitted and root in floors, 'invalid admitted connection graph')
    _require(type(allowance) is int and 0 <= allowance <= 512
             and len(floors) + allowance <= 512, 'invalid original corridor allowance')
    for point in admitted:
        work()
        _require(type(point) is tuple and len(point) == 3
                 and all(type(n) is int and 1 <= n <= 32766 for n in point)
                 and point[2] == root[2], 'invalid same-level admitted cell')

    # A whole floor component is already connected for free. Treating its every
    # tile as a separate terminal would add exponential cost without knowledge.
    pending, components = set(floors), []
    while pending:
        work()
        if len(components) >= MAX_COMPONENTS:
            raise ComponentLimit('minimum room connection component allowance exhausted')
        seed = min(pending)
        component, queue = {seed}, deque([seed])
        pending.remove(seed)
        while queue:
            work()
            for neighbor in _neighbors(queue.popleft()):
                work()
                if neighbor in pending:
                    pending.remove(neighbor)
                    component.add(neighbor)
                    queue.append(neighbor)
        components.append(component)

    vertices = tuple(sorted(admitted))
    indices = {p: i for i, p in enumerate(vertices)}
    weights = tuple(int(p not in floors) for p in vertices)
    graph = []
    for point in vertices:
        work()
        adjacent = []
        for neighbor in _neighbors(point):
            work()
            if neighbor in indices:
                adjacent.append(indices[neighbor])
        graph.append(tuple(adjacent))
    representatives = tuple(min(component) for component in components)
    terminals = tuple(frozenset(indices[p] for p in component) for component in components)
    vertex_count, subset_count = len(vertices), 1 << len(components)
    # Fixed-width arrays, not millions of boxed tuples or retained path copies.
    width = array('i').itemsize
    table_bytes = 2 * (subset_count - 1) * vertex_count * width
    _require(table_bytes <= MAX_TABLE_BYTES, 'minimum connection table exceeds byte bound')
    infinity = allowance + 1
    costs, traces = [None] * subset_count, [None] * subset_count
    for mask in range(1, subset_count):
        work()
        cost = array('i', [infinity]) * vertex_count
        trace = array('i', [0]) * vertex_count
        costs[mask], traces[mask] = cost, trace
        if mask & (mask - 1) == 0:
            for vertex in sorted(terminals[mask.bit_length() - 1]):
                work()
                cost[vertex] = 0
        else:
            split, splits = (mask - 1) & mask, []
            while split:
                work()
                if split < (mask ^ split):
                    splits.append(split)
                split = (split - 1) & mask
            for split in sorted(splits):
                left, right = costs[split], costs[mask ^ split]
                for vertex in range(vertex_count):
                    work()
                    if left[vertex] == infinity or right[vertex] == infinity:
                        continue
                    value = left[vertex] + right[vertex] - weights[vertex]
                    if value < cost[vertex]:
                        cost[vertex], trace[vertex] = value, -split
        queue = []
        for vertex, value in enumerate(cost):
            work()
            if value < infinity:
                queue.append((value, vertex))
        heapq.heapify(queue)
        while queue:
            work()
            value, vertex = heapq.heappop(queue)
            if value != cost[vertex]:
                continue
            for following in graph[vertex]:
                work()
                candidate = value + weights[following]
                if candidate < cost[following]:
                    cost[following], trace[following] = candidate, vertex + 1
                    heapq.heappush(queue, (candidate, following))

    mask, vertex = subset_count - 1, indices[root]
    optimum = costs[mask][vertex]
    if optimum == infinity:
        raise NoConnection('no complete room connection fits corridor tile allowance')
    stack, visited, added = [(mask, vertex)], set(), set()
    while stack:
        work()
        mask, vertex = stack.pop()
        if (mask, vertex) in visited:
            continue
        visited.add((mask, vertex))
        if weights[vertex]:
            added.add(vertices[vertex])
        trace = traces[mask][vertex]
        if trace > 0:
            stack.append((mask, trace - 1))
        elif trace < 0:
            split = -trace
            stack.append((mask ^ split, vertex))
            stack.append((split, vertex))
        else:
            _require(mask & (mask - 1) == 0
                     and vertex in terminals[mask.bit_length() - 1],
                     'minimum connection reconstruction lacks a terminal')
    _require(len(added) == optimum and not added & floors and added <= admitted,
             'minimum connection reconstruction differs from exact cost')
    work()
    return Minimum(tuple(sorted(added)), representatives, subset_count - 1,
                   optimum, table_bytes)
