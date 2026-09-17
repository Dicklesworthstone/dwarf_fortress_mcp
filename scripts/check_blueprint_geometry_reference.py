#!/usr/bin/env python3
"""Independent finite geometry reference; does not compile or execute Rust."""
from collections import deque
import json


def cells(rect):
    x0, y0, x1, y1 = rect
    assert x0 <= x1 and y0 <= y1
    return {(x, y) for y in range(y0, y1 + 1) for x in range(x0, x1 + 1)}


def disjoint_union(parts):
    result = set()
    for part in parts:
        points = cells(part)
        assert not result.intersection(points), "overlapping layout parts"
        result.update(points)
    assert len(parts) <= 64 and len(result) <= 16384
    return result


def moat(width, height, gap):
    parts = [(0, 1, 0, height - 2), (width - 1, 1, width - 1, height - 2),
             (0, height - 1, width - 1, height - 1)]
    if gap:
        start = 1 + (width - 2 - gap) // 2
        parts += [(0, 0, start - 1, 0), (start + gap, 0, width - 1, 0)]
    else:
        parts.append((0, 0, width - 1, 0))
    return disjoint_union(parts)


def cluster(count, width, height, columns):
    parts, rooms = [], []
    rows = (count - 1) // columns + 1
    for row in range(rows):
        n = min(columns, count - row * columns)
        y = row * (height + 3)
        for column in range(n):
            x = column * (width + 1)
            room = (x, y, x + width - 1, y + height - 1)
            rooms.append(room)
            door = x + (width - 1) // 2
            parts += [room, (door, y + height, door, y + height)]
        parts.append((-1, y + height + 1, n * (width + 1) - 2, y + height + 1))
    start = (-2, height + 1)
    parts.append((*start, -2, (rows - 1) * (height + 3) + height + 1))
    result = disjoint_union(parts)
    seen, queue = {start}, deque([start])
    while queue:
        x, y = queue.popleft()
        for p in [(x-1, y), (x+1, y), (x, y-1), (x, y+1)]:
            if p in result and p not in seen:
                seen.add(p)
                queue.append(p)
    assert seen == result, "disconnected excavation"
    for x0, y0, x1, y1 in rooms:
        for x in range(x0, x1 + 1):
            assert ((x, y1 + 1) in result) == (x == x0 + (width - 1) // 2)
    return result


def main():
    counts = {"moats": 0, "bedrooms": 0, "workshops": 0}
    for width in range(3, 17):
        for height in range(3, 17):
            for gap in range(width - 1):
                boundary = cells((0, 0, width-1, height-1)) - cells((1, 1, width-2, height-2))
                start = (width - gap) // 2
                crossing = {(x, 0) for x in range(start, start + gap)}
                assert moat(width, height, gap) == boundary - crossing
                counts["moats"] += 1
    for count in range(1, 25):
        for width in range(1, 8):
            for height in range(1, 8):
                cluster(count, width, height, 4)
                counts["bedrooms"] += 1
        points = cluster(count, 5, 5, 24)
        for wall in range(1, count):
            assert all((wall * 6 - 1, y) not in points for y in range(5))
        counts["workshops"] += 1
    print(json.dumps({"evidence": "independent_python_geometry_reference_only", "cases": counts,
                      "total": sum(counts.values()), "rust_executed": False}, sort_keys=True))


if __name__ == "__main__":
    main()
