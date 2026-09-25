"""Bounded sparse terrain goals evaluated against one unchanged map/1.5 capture.

This is a read-only goal model, not a designation plan or mutation authority.
Every target requires its exact normalized shape, zero liquid depth and no dig
designation. A matching wall is a sampled wall, not proof it was preserved
between reads; matching stairs are not a pathfinding or construction proof.
"""
from __future__ import annotations

from dataclasses import dataclass, field, replace
import hashlib
import json

import excavation_observer as e

SCHEMA = 'dfmcp.excavation-blueprint/1'
GOAL_FORMAT = 'dfmcp.excavation-blueprint-goal/1'
MAX_PARTS = 32
MAX_TARGETS = 512
MAX_VOLUME = 1024
MAX_SPEC_BYTES = 16384
MAX_SAMPLE_BYTES = 575 + 20 * MAX_VOLUME
# Frozen normalized tags from map/1.5, not native DF tiletype enum values.
SHAPES = ('other', 'empty', 'wall', 'floor', 'ramp', 'ramp_top',
          'stair_up', 'stair_down', 'stair_up_down')


@dataclass(frozen=True)
class Part:
    region: e.Region
    shape: str

    def __post_init__(self) -> None:
        e.require(type(self.region) is e.Region, 'invalid blueprint part region')
        e.require(type(self.shape) is str and self.shape in SHAPES[1:],
                  'unsupported blueprint target shape')
        e.require(self.region.volume <= MAX_TARGETS, 'blueprint part exceeds target bound')

    def json(self) -> dict:
        return {'region': self.region.json(), 'shape': self.shape}

    @classmethod
    def from_json(cls, value: object) -> Part:
        e.require(type(value) is dict and set(value) == {'region', 'shape'}, 'invalid blueprint part')
        return cls(e.Region.from_json(value['region']), value['shape'])


@dataclass(frozen=True)
class Blueprint:
    parts: tuple[Part, ...]
    region: e.Region = field(init=False)
    # (native x-fast index in region, required shape tag), in native index order.
    targets: tuple[tuple[int, int], ...] = field(init=False, repr=False)

    def __post_init__(self) -> None:
        e.require(type(self.parts) is tuple and 1 <= len(self.parts) <= MAX_PARTS
                  and all(type(part) is Part for part in self.parts), 'invalid blueprint parts')
        e.require(sum(part.region.volume for part in self.parts) <= MAX_TARGETS,
                  'blueprint exceeds target bound')
        origin = tuple(min(part.region.origin[axis] for part in self.parts) for axis in range(3))
        end = tuple(max(part.region.origin[axis] + part.region.size[axis]
                        for part in self.parts) for axis in range(3))
        region = e.Region(origin, tuple(high - low for low, high in zip(origin, end)))
        e.require(region.volume <= MAX_VOLUME, 'blueprint requires an oversized coherent capture')
        # Validate aggregate extents before expanding any part. Overlap is refused
        # even for equal shapes; do not silently discard conflicting instructions.
        targets = {}
        sx, sy, _ = region.size
        for part in self.parts:
            px, py, pz = part.region.origin
            dx, dy, dz = part.region.size
            shape = SHAPES.index(part.shape)
            for z in range(pz, pz + dz):
                for y in range(py, py + dy):
                    for x in range(px, px + dx):
                        index = ((z - origin[2]) * sy + y - origin[1]) * sx + x - origin[0]
                        e.require(index not in targets, 'overlapping blueprint parts')
                        targets[index] = shape
        ordered_parts = tuple(sorted(self.parts, key=lambda p: (
            p.region.origin[::-1], p.region.size[::-1], p.shape)))
        object.__setattr__(self, 'parts', ordered_parts)
        object.__setattr__(self, 'region', region)
        object.__setattr__(self, 'targets', tuple(sorted(targets.items())))

    def json(self) -> dict:
        return {'schema': SCHEMA, 'parts': [part.json() for part in self.parts]}

    @classmethod
    def from_json(cls, value: object) -> Blueprint:
        e.require(type(value) is dict and set(value) == {'schema', 'parts'}
                  and value['schema'] == SCHEMA and type(value['parts']) is list
                  and 1 <= len(value['parts']) <= MAX_PARTS, 'invalid blueprint specification')
        return cls(tuple(Part.from_json(part) for part in value['parts']))

    @classmethod
    def decode(cls, raw: bytes) -> Blueprint:
        e.require(type(raw) is bytes and 1 <= len(raw) <= MAX_SPEC_BYTES, 'blueprint byte bound exceeded')
        # Nesting is not part of this closed schema. Bound it before json.loads
        # so even an otherwise small deeply nested input cannot recurse freely.
        depth, quoted, escaped = 0, False, False
        for byte in raw:
            if quoted:
                if escaped:
                    escaped = False
                elif byte == 92:
                    escaped = True
                elif byte == 34:
                    quoted = False
            elif byte == 34:
                quoted = True
            elif byte in (91, 123):
                depth += 1
                e.require(depth <= 8, 'blueprint nesting bound exceeded')
            elif byte in (93, 125):
                depth -= 1
        try:
            return cls.from_json(json.loads(raw.decode('utf-8'), object_pairs_hook=e.unique_object))
        except (UnicodeError, ValueError, TypeError, KeyError, RecursionError) as cause:
            raise e.Rejected('invalid blueprint JSON') from cause

    @property
    def digest(self) -> str:
        # Identity covers the semantic mask, not incidental rectangle ordering or
        # splitting. World/source identity is bound separately by the goal/journal.
        value = {'region': self.region.json(), 'targets': self.targets}
        return hashlib.sha256(b'dfmcp-excavation-blueprint/1\0' + e.canonical(value)).hexdigest()

    def position(self, index: int) -> tuple[int, int, int]:
        e.integer(index, 0, self.region.volume - 1)
        sx, sy, _ = self.region.size
        x, y, z = self.region.origin
        return x + index % sx, y + index // sx % sy, z + index // (sx * sy)


def _classify(blueprint: Blueprint, capture: e.Capture, limit: int) -> dict:
    """Internal: capture was decoded against this exact blueprint region."""
    counts = dict.fromkeys(('matched', 'mismatched', 'hidden', 'missing',
                           'wrong_shape', 'wet', 'designated'), 0)
    remaining = []
    for index, expected in blueprint.targets:
        tile = capture.tiles[index]
        reasons = []
        observed = None
        if tile.presence != 2:
            status = 'hidden' if tile.presence == 1 else 'missing'
        else:
            _, shape, depth, _, _, dig, *_ = tile.attributes
            observed = {'shape': SHAPES[shape], 'liquid_depth': depth, 'dig_designation': dig}
            for reason, mismatch in (('wrong_shape', shape != expected), ('wet', depth != 0),
                                     ('designated', dig != 0)):
                if mismatch:
                    counts[reason] += 1
                    reasons.append(reason)
            status = 'mismatched' if reasons else 'matched'
        counts[status] += 1
        if status != 'matched' and len(remaining) < limit:
            remaining.append({'position': list(blueprint.position(index)), 'required_shape': SHAPES[expected],
                              'status': status, 'observed': observed, 'mismatches': reasons})
    count = len(blueprint.targets)
    return {'blueprint_digest': blueprint.digest, 'counts': counts,
            'target_tiles': count, 'captured_tiles': blueprint.region.volume,
            'unselected_tiles': blueprint.region.volume - count, 'remaining': remaining,
            'remaining_omitted': count - counts['matched'] - len(remaining)}


def diagnose(blueprint: Blueprint, capture: e.Capture, limit: int = 16) -> dict:
    """Exact selected-cell counts and bounded whole-row deficits, never causes."""
    e.integer(limit, 0, 64)
    capture = e.decode_capture(capture.raw, capture.manifest, blueprint.region)
    return {**_classify(blueprint, capture, limit), 'observation_witness': capture.witness,
            'observed_tick': capture.tick, 'unselected_cells_evaluated': False,
            'safety_proven': False, 'mining_action_completed_proven': False}


@dataclass(frozen=True)
class BlueprintGoal:
    blueprint: Blueprint
    folder: str
    site: int
    deadline_tick: int
    stable_ticks: int = 10
    required_samples: int = 2
    max_gap_ticks: int = 1200

    def __post_init__(self) -> None:
        e.require(type(self.blueprint) is Blueprint, 'invalid goal blueprint')
        e.require(isinstance(self.folder, str), 'invalid goal fortress')
        e.text(self.folder.encode('utf-8'), 512)
        e.integer(self.site, 0, 2**31 - 1)
        e.integer(self.deadline_tick, 0, e.MAX_TICK)
        e.integer(self.stable_ticks, 0, 403200)
        e.integer(self.required_samples, 1, 128)
        e.integer(self.max_gap_ticks, 1, 403200)

    @property
    def region(self) -> e.Region:
        return self.blueprint.region

    def json(self) -> dict:
        return {'blueprint': self.blueprint.json(), 'folder': self.folder, 'site': self.site,
                'deadline_tick': self.deadline_tick, 'stable_ticks': self.stable_ticks,
                'required_samples': self.required_samples, 'max_gap_ticks': self.max_gap_ticks}

    @classmethod
    def from_json(cls, value: object) -> BlueprintGoal:
        e.require(type(value) is dict and set(value) == {'blueprint', 'folder', 'site',
                  'deadline_tick', 'stable_ticks', 'required_samples', 'max_gap_ticks'}, 'invalid blueprint goal')
        return cls(**{**value, 'blueprint': Blueprint.from_json(value['blueprint'])})


def advance(goal: BlueprintGoal, prior: e.Progress | None, capture: e.Capture) -> e.Progress:
    """All selected targets must match together at each qualifying sample.

    Prior progress is an internal transition result, never deserialized success.
    Journal replay must call this function on every retained native sample.
    """
    capture = e.decode_capture(capture.raw, capture.manifest, goal.region)
    if prior is not None:
        e.require(prior.status not in e.TERMINAL, 'terminal goal is immutable')
        if capture.binding() != prior.first.binding() or capture.tick < prior.latest.tick:
            return replace(prior, status='invalidated', streak=0, since_tick=None,
                           interruption='source_identity_or_clock_changed')
    else:
        e.require(capture.folder == goal.folder and capture.site == goal.site,
                  'observed fortress differs from explicit goal')
        e.require(capture.tick <= goal.deadline_tick, 'goal deadline already passed at creation')
    first = capture if prior is None else prior.first
    observations = 1 if prior is None else prior.observations + 1
    if capture.tick > goal.deadline_tick:
        return e.Progress(first, capture, 'expired', observations=observations)
    counts = _classify(goal.blueprint, capture, 0)['counts']
    if counts['hidden'] or counts['missing']:
        return e.Progress(first, capture, 'unknown', observations=observations)
    if counts['mismatched']:
        return e.Progress(first, capture, 'pending', observations=observations)
    gap = prior is not None and capture.tick - prior.latest.tick > goal.max_gap_ticks
    continuing = prior is not None and prior.streak > 0 and not gap
    since = prior.since_tick if continuing else capture.tick
    streak = prior.streak + int(capture.tick > prior.latest.tick) if continuing else 1
    satisfied = streak >= goal.required_samples and capture.tick - since >= goal.stable_ticks
    return e.Progress(first, capture, 'satisfied' if satisfied else 'stabilizing', streak, since,
                      observations, 'sample_gap_reset' if gap else None)
