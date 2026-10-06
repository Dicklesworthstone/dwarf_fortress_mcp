"""Retain the exact completed whole-room journal without copying its full history.

The reference is NOT a proof or acquisition attestation. Effect owners must open,
replay and hold the original private journal. No read here creates, repairs,
renews or cancels anything. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re

import excavation_observer as e
import room_terrain_goal as r
from furniture_allocation import Guard, idle
from furniture_plan import canonical, require, unique
from room_provisioning import RoomPlan

SCHEMA = 'dfmcp.room-terrain-origin/1'
MAX_BYTES = 8192
MAX_JOURNAL = 32 * 1024 * 1024
MAX_WORK = 8000000
POLICY_FIELDS = {'deadline_tick', 'stable_ticks', 'required_samples', 'max_gap_ticks'}


def read_json(raw: bytes, maximum: int, depth_limit: int, guard: Guard) -> dict:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum, 'terrain origin byte bound exceeded')
    depth, quoted, escaped = 0, False, False
    for offset, byte in enumerate(raw):
        if offset % 256 == 0:
            guard()
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
            require(depth <= depth_limit, 'terrain origin nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1
    def nonfinite(_value):
        raise ValueError('nonfinite terrain origin value')
    value = json.loads(raw, object_pairs_hook=unique, parse_constant=nonfinite)
    require(type(value) is dict, 'terrain origin object required')
    guard()
    return value


class ReadBudget:
    """Adapt an existing owner budget, never create a second deadline/allowance."""
    def __init__(self, parent):
        self.parent = parent
        if not hasattr(parent, '_terrain_origin_work_left'):
            parent._terrain_origin_work_left = MAX_WORK

    def remaining_ms(self) -> int:
        if hasattr(self.parent, 'remaining_ms'):
            return self.parent.remaining_ms()
        value = int(self.parent.remaining() * 1000)
        require(value >= 1, 'terrain origin shared deadline exhausted')
        return value

    def checkpoint(self) -> None:
        self.remaining_ms()
        require(self.parent._terrain_origin_work_left > 0, 'terrain origin shared work exhausted')
        self.parent._terrain_origin_work_left -= 1
        if hasattr(self.parent, 'checkpoint'):
            self.parent.checkpoint()
        elif hasattr(self.parent, 'work'):
            self.parent.work()


@dataclass(frozen=True)
class TerrainOrigin:
    _raw: bytes

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> TerrainOrigin:
        value = read_json(raw, MAX_BYTES, 6, guard)
        require(set(value) == {'schema', 'journal_path', 'file_identity', 'parent_identity',
            'journal_bytes', 'journal_sha256', 'journal_id', 'journal_head', 'goal_digest',
            'room_plan_digest', 'endpoint', 'source', 'policy', 'completed_tick',
            'matching_since_tick', 'matching_samples'}, 'invalid terrain origin fields')
        require(value['schema'] == SCHEMA, 'wrong terrain origin schema')
        path = value['journal_path']
        require(type(path) is str and '\0' not in path and 1 <= len(path.encode('utf-8')) <= 4096,
                'bounded terrain journal path required')
        p = Path(path)
        require(p.is_absolute() and str(p) == path and '..' not in p.parts and len(p.parts) >= 2,
                'absolute canonical terrain journal path required')
        for key in ('file_identity', 'parent_identity'):
            require(type(value[key]) is list and len(value[key]) == 2, 'invalid terrain file identity')
            for number in value[key]:
                e.integer(number, 0, 2**64 - 1)
        for key in ('journal_sha256', 'journal_id', 'journal_head', 'goal_digest', 'room_plan_digest'):
            require(type(value[key]) is str and re.fullmatch('[0-9a-f]{64}', value[key]) is not None,
                    'invalid terrain origin digest')
        e.integer(value['journal_bytes'], 1, MAX_JOURNAL)
        e.endpoint(value['endpoint'])
        source = value['source']
        require(type(source) is dict and set(source) == {'manifest', 'region', 'folder', 'site', 'dimensions'},
                'invalid terrain origin source')
        e.Manifest.from_json(source['manifest'])
        region = e.Region.from_json(source['region'])
        require(type(source['folder']) is str, 'invalid terrain fortress')
        e.text(source['folder'].encode('utf-8'), 512)
        e.integer(source['site'], 0, 2**31 - 1)
        require(type(source['dimensions']) is list and len(source['dimensions']) == 3,
                'invalid terrain dimensions')
        for start, size, bound in zip(region.origin, region.size, source['dimensions']):
            e.integer(bound, 1, 32768)
            require(start + size <= bound, 'terrain region outside retained map')
        policy = value['policy']
        require(type(policy) is dict and set(policy) == POLICY_FIELDS, 'invalid terrain goal policy')
        e.integer(policy['deadline_tick'], 0, e.MAX_TICK)
        e.integer(policy['stable_ticks'], 0, 403200)
        e.integer(policy['required_samples'], 1, 128)
        e.integer(policy['max_gap_ticks'], 1, 403200)
        tick = e.integer(value['completed_tick'], 0, policy['deadline_tick'])
        since = e.integer(value['matching_since_tick'], 0, tick)
        e.integer(value['matching_samples'], policy['required_samples'], 129)
        require(tick - since >= policy['stable_ticks'], 'incomplete claimed terrain stability')
        require(canonical(value) == raw, 'terrain origin must be canonical')
        guard()
        return cls(raw)

    def json(self) -> dict:
        return json.loads(self._raw)

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-terrain-origin/1\0' + self._raw).hexdigest()

    def goal(self, room_plan: RoomPlan, guard: Guard = idle) -> r.RoomTerrainGoal:
        value = self.decode(self._raw, guard).json()
        goal = r.RoomTerrainGoal(room_plan, **value['policy'], checkpoint=guard)
        require(goal.room_plan.digest == value['room_plan_digest'] and goal.digest == value['goal_digest']
                and goal.region.json() == value['source']['region'], 'terrain goal changed original room intent')
        intent = goal.room_plan.json()['intent']
        require((intent['world_folder'], intent['site']) == (value['source']['folder'], value['source']['site']),
                'terrain goal fortress changed')
        return goal

    @classmethod
    def from_journal(cls, journal, guard: Guard = idle) -> TerrainOrigin:
        import track_excavation as t
        require(type(journal) is t.Journal and not journal.writable, 'read-only terrain journal owner required')
        journal.verify()
        history = journal.history  # Supplied only by the concrete private owner after full replay.
        require(history.profile is t.ROOM_PROFILE and not history.pending_read
                and history.progress.status == 'satisfied', 'complete original room terrain is not satisfied')
        goal, progress = history.goal, history.progress
        value = {'schema': SCHEMA, 'journal_path': str(journal.path),
            'file_identity': list(journal.identity), 'parent_identity': list(journal.parent_identity),
            'journal_bytes': len(journal.raw), 'journal_sha256': hashlib.sha256(journal.raw).hexdigest(),
            'journal_id': history.identity, 'journal_head': history.head, 'goal_digest': goal.digest,
            'room_plan_digest': goal.room_plan.digest, 'endpoint': history.endpoint,
            'source': progress.first.binding(), 'policy': {key: getattr(goal, key) for key in POLICY_FIELDS},
            'completed_tick': progress.latest.tick, 'matching_since_tick': progress.since_tick,
            'matching_samples': progress.streak}
        result = cls.decode(canonical(value), guard)
        guard()
        return result

    def verify(self, journal, room_plan: RoomPlan, guard: Guard = idle) -> None:
        self.goal(room_plan, guard)
        actual = TerrainOrigin.from_journal(journal, guard)
        require(actual._raw == self._raw and journal.history.goal.room_plan.encode() == room_plan.encode(),
                'original terrain journal, custody, goal or history changed')
        guard()

    @contextmanager
    def open(self, room_plan: RoomPlan, budget):
        import track_excavation as t
        shared = ReadBudget(budget)
        self.goal(room_plan, shared.checkpoint)
        with t.open_journal(Path(self.json()['journal_path']), shared) as journal:
            self.verify(journal, room_plan, shared.checkpoint)
            yield journal
            self.verify(journal, room_plan, shared.checkpoint)

    def compact(self) -> dict:
        value = self.json()
        return {'schema': SCHEMA, 'origin_digest': self.digest,
            **{key: value[key] for key in ('journal_id', 'journal_head', 'journal_sha256', 'goal_digest',
                                          'room_plan_digest', 'completed_tick')},
            'original_journal_required': True, 'current_terrain_proven': False,
            'native_acquisition_independently_attested': False}
