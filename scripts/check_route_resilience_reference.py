#!/usr/bin/env python3
"""Algorithm-design oracle for observed edge/internal-tile route redundancy.

This executes a Python mirror, NOT the Rust implementation. Exhaustive minimum
separators are independently enumerated; no flow algorithm is used by the oracle.
No native game, Rust qualification or admission result is implied.
"""
from __future__ import annotations

from collections import deque
from itertools import combinations


def pair(a: int, b: int) -> tuple[int, int]:
    return min(a, b), max(a, b)


def reachable(adj: list[list[int]], source: int, target: int,
              cut: set[tuple[str, int | tuple[int, int]]]) -> bool:
    seen = {source}
    queue = deque([source])
    while queue:
        a = queue.popleft()
        if a == target:
            return True
        for b in adj[a]:
            if (b not in seen and ('tile', b) not in cut
                    and ('edge', pair(a, b)) not in cut):
                seen.add(b)
                queue.append(b)
    return False


def oracle(adj: list[list[int]], source: int, target: int, tiles: bool) -> int:
    if tiles:
        elements = [('tile', i) for i in range(len(adj)) if i not in (source, target)]
        if target in adj[source]:
            elements.append(('edge', pair(source, target)))
    else:
        elements = [('edge', (a, b)) for a in range(len(adj)) for b in adj[a] if a < b]
    for count in range(len(adj[source]) + 1):
        for cut in combinations(elements, count):
            if not reachable(adj, source, target, set(cut)):
                return count
    raise AssertionError('enumerated separator missing')


def certificate(adj: list[list[int]], source: int, target: int, tiles: bool,
                paths: list[list[int]], cuts: list[tuple[str, int | tuple[int, int]]]) -> None:
    assert paths == sorted(paths) and len(cuts) == len(paths)
    assert len(set(cuts)) == len(cuts)
    used_edges, used_tiles = set(), set()
    for path in paths:
        assert len(path) >= 2 and path[0] == source and path[-1] == target
        assert len(set(path)) == len(path)
        for a, b in zip(path, path[1:]):
            assert b in adj[a] and pair(a, b) not in used_edges
            used_edges.add(pair(a, b))
        if tiles:
            for tile in path[1:-1]:
                assert tile not in used_tiles
                used_tiles.add(tile)
    for kind, value in cuts:
        if kind == 'tile':
            assert tiles and value not in (source, target)
            assert isinstance(value, int) and 0 <= value < len(adj)
        else:
            a, b = value
            assert a < b and b in adj[a]
            assert not tiles or (a, b) == pair(source, target)
    assert not reachable(adj, source, target, set(cuts))


def model(adj: list[list[int]], source: int, target: int, tiles: bool):
    n = len(adj)
    entrance = lambda tile: tile * 2 if tiles else tile
    exit_ = lambda tile: tile * 2 + 1 if tiles else tile
    net, links, opposing = [[] for _ in range(n * (2 if tiles else 1))], [], []
    src, dst = exit_(source), entrance(target)

    def add(a, b, meaning):
        i = len(links)
        links.append([a, b, False, meaning])
        net[a].append((b, i, False))
        net[b].append((a, i, True))
        return i

    for a, neighbors in enumerate(adj):
        if tiles and a not in (source, target):
            add(entrance(a), exit_(a), ('tile', a))
        for b in sorted(neighbors):
            if tiles:
                if a != target and b != source:
                    add(exit_(a), entrance(b), ('step', a, b))
            elif a < b:
                f = add(a, b, ('step', a, b))
                r = add(b, a, ('step', b, a))
                opposing.append((f, r))
    for neighbors in net:
        neighbors.sort()

    def search(positive=None):
        parent = [None] * len(net)
        parent[src] = (src, 0, False)
        queue = deque([src])
        while queue:
            a = queue.popleft()
            for arc in net[a]:
                b, i, reverse = arc
                eligible = ((not reverse and positive[i]) if positive is not None
                            else links[i][2] == reverse)
                if parent[b] is None and eligible:
                    parent[b] = arc
                    if b == dst:
                        return parent
                    queue.append(b)
        return parent

    def path(parent):
        out, node = [], dst
        while node != src:
            arc = parent[node]
            assert arc is not None and len(out) < len(net)
            _, i, reverse = arc
            node = links[i][1] if reverse else links[i][0]
            out.append(arc)
        return out[::-1]

    units = 0
    while True:
        parent = search()
        if parent[dst] is None:
            break
        for _, i, reverse in path(parent):
            links[i][2] = not reverse
        units += 1
        assert units <= len(adj[source])
    for a, b in opposing:
        if links[a][2] and links[b][2]:
            links[a][2] = links[b][2] = False
    residual = search()
    assert residual[dst] is None
    cuts = set()
    for a, b, flow, meaning in links:
        if residual[a] is None or residual[b] is not None:
            continue
        assert flow
        if meaning[0] == 'tile':
            cuts.add(meaning)
        else:
            _, u, v = meaning
            if not tiles or pair(u, v) == pair(source, target):
                cuts.add(('edge', pair(u, v)))
            else:
                cuts.add(('tile', min(i for i in (u, v) if i not in (source, target))))
    remaining = [link[2] for link in links]
    paths = []
    for _ in range(units):
        parent = search(remaining)
        assert parent[dst] is not None
        route = [source]
        for _, i, _ in path(parent):
            remaining[i] = False
            if links[i][3][0] == 'step':
                route.append(links[i][3][2])
        paths.append(route)
    paths.sort()
    cuts = sorted(cuts, key=lambda cut: (cut[0] != 'tile', cut[1]))
    certificate(adj, source, target, tiles, paths, cuts)
    return paths, cuts


def exhaustive_graphs() -> int:
    checked = 0
    for n in range(2, 6):
        edges = list(combinations(range(n), 2))
        for mask in range(1 << len(edges)):
            adj = [[] for _ in range(n)]
            for i, (a, b) in enumerate(edges):
                if mask & (1 << i):
                    adj[a].append(b)
                    adj[b].append(a)
            for source in range(n):
                for target in range(n):
                    if source == target:
                        continue
                    for tiles in (False, True):
                        paths, _ = model(adj, source, target, tiles)
                        assert len(paths) == oracle(adj, source, target, tiles), (n, mask, source, target, tiles)
                        checked += 1
    return checked


def exhaustive_maps() -> int:
    checked = 0
    for mask in range(1 << 9):
        if mask & 1 or mask & (1 << 8):
            continue
        adj = [[] for _ in range(9)]
        for a in range(9):
            for b in range(9):
                if (not mask & (1 << a) and not mask & (1 << b)
                        and abs(a % 3 - b % 3) + abs(a // 3 - b // 3) == 1):
                    adj[a].append(b)
        for tiles in (False, True):
            paths, _ = model(adj, 0, 8, tiles)
            assert len(paths) == oracle(adj, 0, 8, tiles)
            checked += 1
    return checked


def adversarial_examples() -> None:
    # Two cycles sharing one articulation tile: edge redundancy is two, internal
    # tile redundancy is one. The goal is not an endpoint-exclusion workaround.
    adj = [[1, 2], [0, 3], [0, 3], [1, 2, 4, 5], [3, 6], [3, 6], [4, 5]]
    assert len(model(adj, 0, 6, False)[0]) == 2
    assert model(adj, 0, 6, True)[1] == [('tile', 3)]
    # Adjacent terminals: the direct edge cannot be cut by internal tile removal.
    adj = [[1, 2], [0, 2], [0, 1]]
    assert model(adj, 0, 1, True)[1] == [('tile', 2), ('edge', (0, 1))]
    # Long corridor, no recursive DFS or quadratic set of paths.
    n = 8255
    adj = [[j for j in (i - 1, i + 1) if 0 <= j < n] for i in range(n)]
    for tiles in (False, True):
        paths, cuts = model(adj, 0, n - 1, tiles)
        assert paths == [list(range(n))] and len(cuts) == 1
    # Up/down stair-lattice abstraction, both terminals of degree six.
    size = 4
    adj = [[] for _ in range(size ** 3)]
    for a in range(size ** 3):
        xyz = (a % size, (a // size) % size, a // (size * size))
        for axis in range(3):
            for delta in (-1, 1):
                p = list(xyz)
                p[axis] += delta
                if 0 <= p[axis] < size:
                    adj[a].append(p[0] + size * p[1] + size * size * p[2])
    for tiles in (False, True):
        assert len(model(adj, 21, 42, tiles)[0]) == 6


def main() -> None:
    print(f'Python graph/model cases: {exhaustive_graphs()} passed')
    print(f'Python obstacle-map/model cases: {exhaustive_maps()} passed')
    adversarial_examples()
    print('Articulation, adjacent endpoints, long corridor and six-path examples passed')
    print('Algorithm-design evidence only; Rust implementation not executed by this script.')


if __name__ == '__main__':
    main()
