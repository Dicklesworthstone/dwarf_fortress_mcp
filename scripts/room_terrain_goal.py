"""Whole-original-room terrain goals; no excavation or furniture authority.

Every original floor AND separating bedroom wall must hold in the same capture
and the same advancing-tick streak. Native-format bytes are re-decoded, never
caller-supplied tile objects or a residual excavation's success assertion.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10; remain open).
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass, field, replace
import hashlib

import excavation_observer as e
import room_terrain as terrain
from furniture_allocation import Guard, idle
from room_provisioning import MAX_PLAN_BYTES, RoomPlan

GOAL_FORMAT = 'dfmcp.room-terrain-goal/1'
PROGRESS_SCHEMA = 'dfmcp.room-terrain-progress/1'
POLICY = 'dfmcp.whole-room-floor-and-wall-stability/1'
MAX_GOAL_BYTES = MAX_PLAN_BYTES + 1024
MAX_DEFICITS = 64
CATEGORIES = ('matched', 'wrong_shape', 'liquid', 'designated', 'occupied_wall', 'hidden', 'missing')


@dataclass(frozen=True)
class RoomTerrainGoal:
    room_plan: RoomPlan
    deadline_tick: int
    stable_ticks: int = 10
    required_samples: int = 2
    max_gap_ticks: int = 1200
    checkpoint: InitVar[Guard | None] = None
    _selection: terrain.Selection = field(init=False, repr=False, compare=False)

    def __post_init__(self, checkpoint: Guard | None) -> None:
        checkpoint = idle if checkpoint is None else checkpoint
        checkpoint()
        e.integer(self.deadline_tick, 0, e.MAX_TICK)
        e.integer(self.stable_ticks, 0, 403200)
        e.integer(self.required_samples, 1, 128)
        e.integer(self.max_gap_ticks, 1, 403200)
        selected = terrain.selection(self.room_plan, checkpoint)
        object.__setattr__(self, 'room_plan', selected.plan)
        object.__setattr__(self, '_selection', selected)
        checkpoint()

    @property
    def region(self) -> e.Region:
        return self._selection.region

    def json(self) -> dict:
        return {'schema': GOAL_FORMAT, 'policy': POLICY, 'room_plan': self.room_plan.json(),
                'deadline_tick': self.deadline_tick, 'stable_ticks': self.stable_ticks,
                'required_samples': self.required_samples, 'max_gap_ticks': self.max_gap_ticks}

    @classmethod
    def from_json(cls, value: object, guard: Guard = idle) -> RoomTerrainGoal:
        guard()
        e.require(type(value) is dict and set(value) == {'schema', 'policy', 'room_plan',
            'deadline_tick', 'stable_ticks', 'required_samples', 'max_gap_ticks'}
            and value['schema'] == GOAL_FORMAT and value['policy'] == POLICY, 'invalid whole-room goal')
        raw = e.canonical(value)
        e.require(len(raw) <= MAX_GOAL_BYTES, 'whole-room goal byte bound exceeded')
        plan = RoomPlan.decode(e.canonical(value['room_plan']), guard)
        return cls(plan, value['deadline_tick'], value['stable_ticks'], value['required_samples'],
                   value['max_gap_ticks'], checkpoint=guard)

    def validated(self, guard: Guard = idle) -> RoomTerrainGoal:
        # Rebuild even an in-process caller's derived selection and compiled plan.
        return RoomTerrainGoal(self.room_plan, self.deadline_tick, self.stable_ticks,
                               self.required_samples, self.max_gap_ticks, checkpoint=guard)

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-terrain-goal/1\0' + e.canonical(self.json())).hexdigest()


def decode(goal: RoomTerrainGoal, capture: e.Capture, guard: Guard) -> e.Capture:
    e.require(type(capture) is e.Capture and type(capture.raw) is bytes
              and 1 <= len(capture.raw) <= terrain.MAX_CAPTURE_BYTES, 'invalid whole-room capture')
    e.require(type(capture.manifest) is e.Manifest, 'invalid whole-room source manifest')
    manifest = e.Manifest.from_json(capture.manifest.json())
    guard()
    observed = e.decode_capture(capture.raw, manifest, goal.region)
    guard()
    return observed


def _diagnose(goal: RoomTerrainGoal, capture: e.Capture, limit: int, guard: Guard) -> dict:
    selected = goal._selection
    sx, sy, _ = selected.region.size
    ox, oy, oz = selected.region.origin
    counts, rows, occupied_floors = {}, [], 0
    for domain, mask, expected in (('floors', selected.floors, 3), ('required_walls', selected.walls, 2)):
        categories = dict.fromkeys(CATEGORIES, 0)
        for x, y, z in sorted(mask, key=terrain.order):
            guard()
            tile = capture.tiles[((z - oz) * sy + y - oy) * sx + x - ox]
            if tile.presence != 2:
                reason = 'hidden' if tile.presence == 1 else 'missing'
            else:
                _, shape, depth, magma, _, dig, building, units, *_ = tile.attributes
                occupied_floors += int(domain == 'floors' and bool(building or units))
                reason = ('liquid' if depth or magma else 'designated' if dig else
                          'wrong_shape' if shape != expected else
                          'occupied_wall' if domain == 'required_walls' and (building or units) else 'matched')
            categories[reason] += 1
            if reason != 'matched' and len(rows) < limit:
                rows.append({'domain': domain, 'coordinate': [x, y, z], 'reason': reason,
                             'required_shape': 'floor' if expected == 3 else 'wall'})
        counts[domain] = categories
    total = len(selected.floors) + len(selected.walls)
    matched = sum(c['matched'] for c in counts.values())
    return {'policy': POLICY, 'room_plan_digest': goal.room_plan.digest,
            'counts': counts, 'floor_tiles': len(selected.floors), 'required_wall_tiles': len(selected.walls),
            'evaluated_tiles': total, 'captured_tiles': selected.region.volume,
            'unselected_tiles': selected.region.volume - total,
            'occupied_floor_tiles': occupied_floors,
            'deficits': {'count': total - matched, 'rows': rows, 'omitted': total - matched - len(rows)},
            'all_required_shapes_at_sample': matched == total,
            'observation_witness': capture.witness, 'observed_tick': capture.tick,
            'unselected_cells_evaluated': False, 'furniture_placement_eligibility_proven': False,
            'continuous_wall_preservation_proven': False, 'room_completion_proven': False}


def diagnose(goal: RoomTerrainGoal, capture: e.Capture, limit: int = MAX_DEFICITS,
             guard: Guard = idle) -> dict:
    e.integer(limit, 0, MAX_DEFICITS)
    e.require(type(goal) is RoomTerrainGoal, 'exact whole-room goal required')
    goal = goal.validated(guard)
    capture = decode(goal, capture, guard)
    intent = goal.room_plan.json()['intent']
    e.require((capture.folder, capture.site) == (intent['world_folder'], intent['site']),
              'room observation differs from original fortress')
    result = _diagnose(goal, capture, limit, guard)
    guard()
    return result


def advance(goal: RoomTerrainGoal, prior: e.Progress | None, capture: e.Capture,
            guard: Guard = idle) -> e.Progress:
    """Prior is an internal reducer result; durable replay recomputes every sample."""
    e.require(type(goal) is RoomTerrainGoal, 'exact whole-room goal required')
    goal = goal.validated(guard)
    capture = decode(goal, capture, guard)
    if prior is not None:
        e.require(type(prior) is e.Progress and prior.status not in e.TERMINAL,
                  'terminal whole-room goal is immutable')
        if capture.binding() != prior.first.binding() or capture.tick < prior.latest.tick:
            return replace(prior, status='invalidated', streak=0, since_tick=None,
                           interruption='source_identity_or_clock_changed')
    else:
        intent = goal.room_plan.json()['intent']
        e.require((capture.folder, capture.site) == (intent['world_folder'], intent['site']),
                  'room observation differs from original fortress')
        e.require(capture.tick <= goal.deadline_tick, 'room goal deadline already passed')
    first = capture if prior is None else prior.first
    observations = 1 if prior is None else prior.observations + 1
    if capture.tick > goal.deadline_tick:
        return e.Progress(first, capture, 'expired', observations=observations)
    diagnosis = _diagnose(goal, capture, 0, guard)
    if any(c['hidden'] or c['missing'] for c in diagnosis['counts'].values()):
        return e.Progress(first, capture, 'unknown', observations=observations)
    if not diagnosis['all_required_shapes_at_sample']:
        return e.Progress(first, capture, 'pending', observations=observations)
    gap = prior is not None and capture.tick - prior.latest.tick > goal.max_gap_ticks
    continuing = prior is not None and prior.streak > 0 and not gap
    since = prior.since_tick if continuing else capture.tick
    streak = prior.streak + int(capture.tick > prior.latest.tick) if continuing else 1
    satisfied = streak >= goal.required_samples and capture.tick - since >= goal.stable_ticks
    guard()
    return e.Progress(first, capture, 'satisfied' if satisfied else 'stabilizing', streak, since,
                      observations, 'sample_gap_reset' if gap else None)
