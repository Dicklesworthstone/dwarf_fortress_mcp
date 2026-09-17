#!/usr/bin/env python3
"""Independent model/oracle checks; this does NOT execute Rust or DFHack.

The expansion reference is compared to exhaustive batch-vector enumeration on
all 4,096 small DAG/yield/stock configurations. Extra cases check shared stock,
final-stock minima, complete deficits, cycles, and the iterative depth bound.
"""
from itertools import product
import hashlib
import json
from pathlib import Path


def expand(recipes, goals, stock, max_orders=250):
    active = {}
    nodes = set(goals)
    rounds = 0
    while True:
        rounds += 1
        incoming = dict.fromkeys(nodes, 0)
        for _, inputs in active.values():
            for token in inputs:
                incoming[token] += 1
        ready = {token for token, n in incoming.items() if not n}
        order = []
        while ready:
            token = min(ready)
            ready.remove(token)
            order.append(token)
            if token in active:
                for dependency in active[token][1]:
                    incoming[dependency] -= 1
                    if not incoming[dependency]:
                        ready.add(dependency)
        if len(order) != len(nodes):
            raise ValueError("active cycle")
        required, batches, pending = dict(goals), {}, []
        for token in order:
            missing = max(0, required.get(token, 0) - stock.get(token, 0))
            if not missing:
                continue
            if token not in active:
                if token in recipes:
                    pending.append(token)
                continue
            yield_units, inputs = active[token]
            batches[token] = (missing + yield_units - 1) // yield_units
            for dependency, amount in inputs.items():
                required[dependency] = required.get(dependency, 0) + amount * batches[token]
        if not pending:
            deficits = {token: required[token] - stock.get(token, 0)
                        for token in required
                        if token not in batches and required[token] > stock.get(token, 0)}
            return batches, deficits, required, rounds
        for token in pending:
            if len(active) >= max_orders:
                raise ValueError("order budget")
            active[token] = recipes[token]
            nodes.update(recipes[token][1])


def brute(recipes, goals, stock):
    """Direct material-balance feasibility, independent of demand expansion."""
    best = None
    for a, b, c in product(range(3), range(4), range(7)):
        counts = {"A": a, "B": b, "C": c}
        balances = dict(stock)
        for token, n in counts.items():
            yield_units, inputs = recipes[token]
            balances[token] = balances.get(token, 0) + yield_units * n
            for dependency, amount in inputs.items():
                balances[dependency] = balances.get(dependency, 0) - amount * n
        if all(balances.get(token, 0) >= minimum for token, minimum in goals.items()) \
                and all(n >= 0 for n in balances.values()):
            candidate = (a + b + c, (a, b, c))
            if best is None or candidate < best:
                best = candidate
    if best is None:
        raise AssertionError("fixture oracle bounds excluded all plans")
    return best[1]


def main():
    names = ("A", "B", "C", "RAW")
    edges = ((0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3))
    cases = 0
    for mask, yield_mask, stock_mask in product(range(64), range(8), range(8)):
        recipes = {names[n]: (1 + ((yield_mask >> n) & 1),
                   {names[to]: 1 for bit, (src, to) in enumerate(edges)
                    if src == n and mask & (1 << bit)}) for n in range(3)}
        stock = {names[n]: (stock_mask >> n) & 1 for n in range(3)} | {"RAW": 32}
        goals = {"A": 2, "B": 1, "C": 1}
        actual, deficits, _, _ = expand(recipes, goals, stock)
        assert not deficits
        assert tuple(actual.get(n, 0) for n in names[:3]) == brute(recipes, goals, stock), \
            (mask, yield_mask, stock_mask)
        reordered = dict(reversed(list(recipes.items())))
        assert expand(reordered, dict(reversed(list(goals.items()))), stock) == \
            expand(recipes, goals, stock)
        cases += 1

    recipes = {"A": (1, {"RAW": 2}), "B": (1, {"RAW": 2})}
    assert expand(recipes, {"A": 1}, {"RAW": 2})[1] == {}
    assert expand(recipes, {"B": 1}, {"RAW": 2})[1] == {}
    assert expand(recipes, {"A": 1, "B": 1}, {"RAW": 2})[1] == {"RAW": 2}
    cases += 3

    recipes = {"A": (1, {"B": 1}), "B": (3, {"RAW": 2})}
    batches, deficits, required, _ = expand(recipes, {"A": 2, "B": 2}, {"B": 1, "RAW": 2})
    assert batches == {"A": 2, "B": 1} and not deficits and required["B"] == 4
    cases += 1
    recipes = {"A": (1, {"X": 2, "Y": 3})}
    assert expand(recipes, {"A": 2, "Z": 1}, {})[1] == {"X": 4, "Y": 6, "Z": 1}
    cases += 1

    recipes = {"A": (1, {"B": 1}), "B": (1, {"A": 1})}
    assert expand(recipes, {"A": 1}, {"B": 1})[0] == {"A": 1}
    try:
        expand(recipes, {"A": 1}, {})
    except ValueError as error:
        assert str(error) == "active cycle"
    else:
        raise AssertionError("active cycle accepted")
    cases += 2

    recipes = {f"N{n:03}": (1, {f"N{n + 1:03}": 1}) for n in range(250)}
    batches, deficits, _, rounds = expand(recipes, {"N000": 1}, {"N250": 1})
    assert len(batches) == 250 and not deficits and rounds == 251
    recipes["N250"] = (1, {"N251": 1})
    try:
        expand(recipes, {"N000": 1}, {"N251": 1})
    except ValueError as error:
        assert str(error) == "order budget"
    else:
        raise AssertionError("order budget exceeded without rejection")
    cases += 2
    print(json.dumps({"status": "passed", "model_cases": cases, "exhaustive_dag_cases": 4096,
                      "rust_executed": False, "native_or_live_executed": False,
                      "reference_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}, sort_keys=True))


if __name__ == "__main__":
    main()
