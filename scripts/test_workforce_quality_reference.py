#!/usr/bin/env python3
"""Independent executable model/oracle, NOT execution or qualification of Rust.

Compares residual vector-potential allocation with exhaustive worker choices and
independently verifies cuts and reduced costs. Standard library only.
"""
from itertools import product
from random import Random

ZERO = (0, 0, 0, 0)


def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def model(capacities, priorities, candidates):
    ids = sorted({c[0] for c in candidates})
    nodes = {w: 1 + len(capacities) + i for i, w in enumerate(ids)}
    sink = 1 + len(capacities) + len(ids)
    arcs = [(0, 1 + d, n, ZERO, None) for d, n in enumerate(capacities)]
    for w, d, effective, nominal, steps in candidates:
        arcs.append((1 + d, nodes[w], sum(capacities), (-priorities[d], -effective, -nominal, steps), (d, w)))
    arcs += [(nodes[w], sink, 1, ZERO, None) for w in ids]
    return ids, nodes, sink, arcs


def reference(capacities, priorities, candidates):
    ids, nodes, sink, arcs = model(capacities, priorities, candidates)
    graph = [[] for _ in range(sink + 1)]
    links = []
    for u, v, cap, cost, key in arcs:
        forward, reverse = len(graph[u]), len(graph[v])
        graph[u].append([v, reverse, cap, cost])
        graph[v].append([u, forward, 0, sub(ZERO, cost)])
        if key is not None:
            links.append((u, forward, key))
    potential = [ZERO] * len(graph)
    for w, d, effective, nominal, steps in candidates:
        potential[nodes[w]] = min(potential[nodes[w]], (-priorities[d], -effective, -nominal, steps))
    potential[sink] = min(potential)
    assigned = 0
    while True:
        distance, parent = [None] * len(graph), [None] * len(graph)
        distance[0] = ZERO
        queue = {(ZERO, 0)}
        while queue:
            label, u = min(queue)
            queue.remove((label, u))
            for i, (v, _, cap, cost) in enumerate(graph[u]):
                if not cap:
                    continue
                reduced = sub(add(cost, potential[u]), potential[v])
                assert reduced >= ZERO
                new = add(label, reduced)
                if distance[v] is None or new < distance[v]:
                    if distance[v] is not None:
                        queue.discard((distance[v], v))
                    distance[v], parent[v] = new, (u, i)
                    queue.add((new, v))
        if distance[sink] is None:
            break
        largest = max(d for d in distance if d is not None)
        potential = [add(p, d if d is not None else largest) for p, d in zip(potential, distance)]
        v, seen = sink, set()
        while v:
            assert v not in seen
            seen.add(v)
            u, i = parent[v]
            edge = graph[u][i]
            graph[u][i][2] -= 1
            graph[v][edge[1]][2] += 1
            v = u
        assigned += 1
    chosen = tuple(key for u, i, key in links if graph[u][i][2] < sum(capacities))
    witness = (potential, [d is not None for d in distance])
    score = verify(capacities, priorities, candidates, chosen, witness)
    assert len(chosen) == assigned
    return score, chosen, witness


def verify(capacities, priorities, candidates, chosen, witness):
    """Rebuild every residual edge from chosen pairs, without solver graph state."""
    ids, nodes, sink, arcs = model(capacities, priorities, candidates)
    potential, side = witness
    assert len(potential) == sink + 1 == len(side) and side[0] and not side[sink]
    assert tuple(sorted(set(chosen))) == chosen
    used = {w for _, w in chosen}
    assert len(used) == len(chosen)
    counts = [sum(d == i for d, _ in chosen) for i in range(len(capacities))]
    assert all(n <= cap for n, cap in zip(counts, capacities))
    allowed = {(d, w) for w, d, *_ in candidates}
    assert set(chosen) <= allowed
    cut, total = 0, ZERO
    for u, v, cap, cost, key in arcs:
        flow = counts[v - 1] if u == 0 else int((key in chosen) if key else (ids[u - 1 - len(capacities)] in used))
        assert 0 <= flow <= cap
        if side[u] and not side[v]:
            cut += cap
        if flow < cap:
            assert sub(add(cost, potential[u]), potential[v]) >= ZERO
            assert not (side[u] and not side[v])
        if flow:
            assert sub(add(sub(ZERO, cost), potential[v]), potential[u]) >= ZERO
            assert not (side[v] and not side[u])
        if key and flow:
            total = add(total, cost)
    assert cut == len(chosen)
    if len(chosen) < sum(capacities):
        deficient = {d for d in range(len(capacities)) if side[1 + d]}
        neighbors = {w for w, d, *_ in candidates if d in deficient}
        assert sum(capacities[d] for d in deficient) - len(neighbors) == sum(capacities) - len(chosen)
    return (-len(chosen), *total)


def exhaustive(capacities, priorities, candidates):
    ids = sorted({w for w, *_ in candidates})
    rows = {(d, w): (e, n, s) for w, d, e, n, s in candidates}
    best = (0, 0, 0, 0, 0)
    for choices in product(range(-1, len(capacities)), repeat=len(ids)):
        selected = [(d, w) for d, w in zip(choices, ids) if d >= 0]
        if any(key not in rows for key in selected):
            continue
        if any(sum(d == i for d, _ in selected) > cap for i, cap in enumerate(capacities)):
            continue
        score = [-len(selected), 0, 0, 0, 0]
        for d, w in selected:
            e, n, s = rows[d, w]
            score[1] -= priorities[d]
            score[2] -= e
            score[3] -= n
            score[4] += s
        best = min(best, tuple(score))
    return best


def main():
    checked = 0
    for graph in range(512):
        rows = [(w + 1, d, (w * 7 + d * 3) % 11, (w * 5 + d) % 7, (w + d * 3) % 5)
                for d in range(3) for w in range(3) if graph & (1 << (d * 3 + w))]
        for bits in range(8):
            priorities = [(bits >> d) & 1 for d in range(3)]
            score, chosen, witness = reference([1] * 3, priorities, rows)
            assert score == exhaustive([1] * 3, priorities, rows), (graph, bits, score)
            assert (score, chosen, witness) == reference([1] * 3, priorities, rows)
            checked += 1
    rng = Random(0xDFA110C)
    for _ in range(512):
        caps = [rng.randint(1, 3) for _ in range(3)]
        priorities = [rng.randint(0, 1000) for _ in caps]
        rows = [(w + 1, d, rng.randint(0, 2**31 - 1), rng.randint(-2**31, 2**31 - 1), rng.randint(0, 2**32 - 1))
                for d in range(3) for w in range(4) if rng.randrange(2)]
        score, _, _ = reference(caps, priorities, rows)
        assert score == exhaustive(caps, priorities, rows)
        checked += 1
    rows = [(1, 0, 10, 10, 0), (2, 0, 1, 1, 0)]
    _, chosen, (potentials, side) = reference([1], [0], rows)
    tampered = [(((0, 2),), (potentials, side)), (chosen, ([ZERO] * len(potentials), side)),
                (chosen, (potentials, [False] * len(side)))]
    for pairs, witness in tampered:
        try:
            verify([1], [0], rows, pairs, witness)
        except AssertionError:
            pass
        else:
            raise AssertionError("tampered certificate accepted")
    print(f"PASS: {checked} exhaustive-oracle comparisons, 4096 deterministic reruns, 3 certificate refusals")
    print("Scope: independent Python reference only; Rust/native/MCP not executed.")


if __name__ == "__main__":
    main()
