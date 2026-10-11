"""Independent executable model/oracle; this does NOT compile or execute Rust."""
from collections import deque
from itertools import product, permutations


def allocate(edges, positions=None):
    positions = list(range(len(edges))) if positions is None else positions
    workers = 1 + max((w for row in edges for w in row), default=-1)
    pm = [None] * len(positions)
    wm = [None] * workers
    for root in range(len(positions)):
        parents = [None] * workers
        visited = {root}
        queue = deque([root])
        success = False
        while queue and not success:
            p = queue.popleft()
            for worker in edges[positions[p]]:
                if parents[worker] is not None:
                    continue
                parents[worker] = p
                occupied = wm[worker]
                if occupied is not None:
                    if occupied not in visited:
                        visited.add(occupied)
                        queue.append(occupied)
                    continue
                free = worker
                while True:
                    target = parents[free]
                    previous = pm[target]
                    pm[target] = free
                    wm[free] = target
                    if previous is None:
                        break
                    wm[previous] = None
                    free = previous
                success = True
                break
    assert len([w for w in pm if w is not None]) == len(set(w for w in pm if w is not None))
    for p, worker in enumerate(pm):
        if worker is not None:
            assert worker in edges[positions[p]] and wm[worker] == p
    return tuple(sum(pm[p] is not None for p, r in enumerate(positions) if r == role) for role in range(len(edges)))


def oracle(edges, counts, workers=3):
    best = (0,) * len(edges)
    for assignment in product(range(-1, len(edges)), repeat=workers):
        totals = [0] * len(edges)
        valid = True
        for worker, role in enumerate(assignment):
            if role >= 0:
                if worker not in edges[role]:
                    valid = False
                    break
                totals[role] += 1
        if valid and all(n <= cap for n, cap in zip(totals, counts)):
            best = max(best, tuple(totals))
    return best


cases = 0
for mask in range(512):
    edges = [[w for w in range(3) if mask & (1 << (w * 3 + r))] for r in range(3)]
    for counts in product(range(3), repeat=3):
        positions = [r for r, count in enumerate(counts) for _ in range(count)]
        actual = allocate(edges, positions)
        expected = oracle(edges, counts)
        assert actual == expected, (mask, counts, actual, expected)
        cases += 1
# Every preference ordering in a dense 3x3 model has the same maximal coverage.
for a, b, c in product(permutations(range(3)), repeat=3):
    assert allocate([a, b, c]) == (1, 1, 1)
    cases += 1
print(f'PASS: {cases} exhaustive assignment/model comparisons (independent Python model, not Rust execution)')
