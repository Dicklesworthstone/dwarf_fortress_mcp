"""Bounded sampled circulation inside the COMPLETE original room floor mask.

This is a conservative cardinal graph, not native unit pathfinding or a route
outside the selected project. Callers own native decoding and source authority.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/07/10).
"""
from __future__ import annotations

from collections import deque
import hashlib
import json
from typing import Callable

POLICY = 'dfmcp.observed-room-circulation/1'
ACCESS_POLICY = 'dfmcp.observed-room-access-circulation/2'
MAX_TILES = 512
MAX_AREAS = 8
MAX_FURNISHINGS = 32
MAX_WORK = 100000
MAX_OUTPUT = 32768
Point = tuple[int, int, int]
Guard = Callable[[], None]


def _idle() -> None:
    """Live owners inject their existing shrinking deadline and authority check."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def _digest(domain: bytes, value: object) -> str:
    return hashlib.sha256(domain + _canonical(value)).hexdigest()


def _point(value: object) -> None:
    _require(type(value) is tuple and len(value) == 3
             and all(type(n) is int and 0 <= n < 32768 for n in value),
             'circulation needs exact bounded coordinate tuples')


def _name(value: object) -> None:
    _require(type(value) is str and 1 <= len(value) <= 48
             and all(c.isascii() and (c.isalnum() or c in '_.-') for c in value),
             'circulation names must be bounded ASCII identifiers')


def _neighbors(point: Point) -> tuple[Point, ...]:
    x, y, z = point
    return (x - 1, y, z), (x, y - 1, z), (x, y + 1, z), (x + 1, y, z)


def assess(floors: frozenset[Point], clear: frozenset[Point],
           uncertain: frozenset[Point], entries: tuple[tuple[str, Point], ...],
           furnishings: tuple[tuple[str, str, Point], ...], *, connected: bool,
           guard: Guard = _idle, max_work: int = MAX_WORK,
           access_point: Point | None = None) -> dict:
    """Evaluate every area entry and furnishing approach in one sampled model.

    `clear` contains observed dry undesignated floors with zero occupancy.
    `uncertain` contains unobserved or unit-occupied potential floor cells.
    Other original floor cells are excluded. Every furnishing target is blocked
    in both graphs, even when its occupancy was absent in the supplied sample.
    True means a clear route exists; unknown means only a potential route exists;
    false means neither exists WITHIN THIS MODEL, never global unreachability.

    With an explicit original `access_point`, every route starts there, not at
    the first room. This is a separate versioned evidence policy. Omitting it
    preserves legacy bytes, work counts and room-local semantics exactly.
    """
    _require(type(connected) is bool and type(max_work) is int
             and 1 <= max_work <= MAX_WORK, 'invalid circulation policy or work bound')
    work = 0

    def charge(units: int = 1) -> None:
        nonlocal work
        work += units
        _require(work <= max_work, 'circulation work allowance exhausted')
        guard()

    charge()
    for mask in (floors, clear, uncertain):
        _require(type(mask) is frozenset and len(mask) <= MAX_TILES,
                 'circulation masks must be bounded immutable sets')
        for point in mask:
            charge()
            _point(point)
    _require(bool(floors) and clear <= floors and uncertain <= floors
             and not clear & uncertain, 'inconsistent circulation partition')
    _require(type(entries) is tuple and 1 <= len(entries) <= MAX_AREAS
             and type(furnishings) is tuple and 1 <= len(furnishings) <= MAX_FURNISHINGS,
             'circulation requires the complete bounded room selection')
    for entry in entries:
        charge()
        _require(type(entry) is tuple and len(entry) == 2, 'invalid area entry')
        _name(entry[0])
        _point(entry[1])
        _require(entry[1] in floors, 'area entry outside original floor mask')
    _require(len({name for name, _ in entries}) == len(entries), 'duplicate area name')
    entries = tuple(sorted(entries))
    by_area = dict(entries)
    for furnishing in furnishings:
        charge()
        _require(type(furnishing) is tuple and len(furnishing) == 3,
                 'invalid furnishing endpoint')
        name, area, point = furnishing
        _name(name)
        _name(area)
        _point(point)
        _require(area in by_area and point in floors, 'unbound furnishing endpoint')
    targets = frozenset(point for _, _, point in furnishings)
    _require(len(targets) == len(furnishings)
             and len({name for name, _, _ in furnishings}) == len(furnishings),
             'duplicate furnishing name or target')
    _require(not targets & {point for _, point in entries}, 'furnishing obstructs an original entry')
    _require({area for _, area, _ in furnishings} == set(by_area), 'area has no original furnishing')
    _require(not connected or len({p[2] for p in floors}) == 1,
             'connected circulation has no implicit stairs')
    if access_point is not None:
        charge()
        _point(access_point)
        _require(connected and access_point in floors and access_point not in targets,
                 'project access must be an unoccupied intended floor of a connected project')
    policy = POLICY if access_point is None else ACCESS_POLICY
    furnishings = tuple(sorted(furnishings))
    walkable = floors - targets
    known = clear & walkable
    possible = (clear | uncertain) & walkable
    # Charge canonicalization/sorting before it can consume unbounded work.
    charge(16 * (len(floors) + len(entries) + len(furnishings)))
    model_inputs = {
        'policy': policy, 'floors': sorted(floors), 'clear': sorted(clear),
        'uncertain': sorted(uncertain), 'entries': entries, 'furnishings': furnishings,
        'connected': connected}
    if access_point is not None:
        model_inputs['access_point'] = access_point
    model = _digest(b'dfmcp-room-circulation-model/1\0', model_inputs)

    def search(start: Point, admitted: frozenset[Point]) -> dict[Point, tuple[Point | None, int]]:
        charge()
        if start not in admitted:
            return {}
        found = {start: (None, 0)}
        pending = deque([start])
        while pending:
            charge()
            current = pending.popleft()
            for neighbor in _neighbors(current):
                charge()
                if neighbor in admitted and neighbor not in found:
                    found[neighbor] = (current, found[current][1] + 1)
                    pending.append(neighbor)
        return found

    # None denotes the independent project root, never an invented ninth area.
    roots = ((None, access_point),) if access_point is not None else ((entries[0],) if connected else entries)
    searches = {area: (search(point, known), search(point, possible)) for area, point in roots}

    def row(kind: str, name: str, area: str | None, endpoint: Point) -> dict:
        charge()
        root = None if access_point is not None else (entries[0][0] if connected else area)
        certain_routes, potential_routes = searches[root]
        approaches = _neighbors(endpoint) if kind == 'furnishing' else (endpoint,)
        chosen, truth = None, 'false'
        route = {}
        for tree, value in ((certain_routes, 'true'), (potential_routes, 'unknown')):
            charge(len(approaches))
            reachable = [p for p in approaches if p in tree]
            if reachable:
                chosen = min(reachable, key=lambda p: (tree[p][1], p))
                route, truth = tree, value
                break
        path = []
        cursor = chosen
        while cursor is not None:
            charge()
            path.append(cursor)
            cursor = route[cursor][0]
        path.reverse()
        return {'kind': kind, 'name': name, 'area': area, 'root_area': root,
                'target': list(endpoint), 'truth': truth,
                'status': {'true': 'clear_route', 'unknown': 'potential_route_only',
                           'false': 'no_route_in_selected_model'}[truth],
                'approach': None if chosen is None else list(chosen),
                'steps': None if chosen is None else len(path) - 1,
                'route_digest': None if chosen is None else _digest(
                    b'dfmcp-room-circulation-route/1\0', {'model': model, 'path': path}),
                'route_is_observed_clear': truth == 'true'}

    rows = [row('area_entry', name, name, point) for name, point in entries]
    rows += [row('furnishing', name, area, point) for name, area, point in furnishings]
    if access_point is not None:
        rows.insert(0, row('project_access', 'project_access', None, access_point))
    counts = {value: sum(r['truth'] == value for r in rows) for value in ('true', 'false', 'unknown')}
    truth = 'false' if counts['false'] else 'unknown' if counts['unknown'] else 'true'
    result = {'policy': policy, 'model_digest': model, 'truth': truth,
              'all_required_routes_clear': truth == 'true', 'connected_project_required': connected,
              'selected_areas': len(entries), 'selected_furnishings': len(furnishings),
              'counts': counts, 'records': rows, 'records_complete': True,
              'floor_cells': len(floors), 'clear_circulation_cells': len(known),
              'uncertain_circulation_cells': len(possible - known),
              'excluded_circulation_cells': len(walkable - possible),
              'native_pathfinding_proven': False, 'outside_project_routes_evaluated': False,
              'existing_fort_access_proven': False, 'continuous_access_proven': False,
              'game_effect_performed': False}
    if access_point is not None:
        result.update(access_point=list(access_point), project_access_required=True,
                      root_kind='original_project_access_point',
                      point_is_existing_fortress_entrance_proven=False)
    charge()
    result['work_units'] = work
    result['evidence_digest'] = _digest(b'dfmcp-room-circulation-evidence/1\0', result)
    _require(len(_canonical(result)) <= MAX_OUTPUT, 'complete circulation diagnosis exceeds output bound')
    guard()
    return result
