"""Whole-room completion for the exact original room-backed route batch.

One existing construction reducer owns temporal stability for both terrain and
all receipt-linked furnishings. No independent successes are latched. The
caller holds original batch custody and supplies freshly acquired raw evidence.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-10.
"""
from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import json
import struct
from types import SimpleNamespace

from furniture_plan import canonical, integer, require
import excavation_observer as map_wire
import construction_plan as construction
from construction_receipt import Cursor, Manifest
import route_furniture_completion as original
import route_furniture_allocation as routes
from route_furniture_frontier import bind_native_selection
import room_route_handoff as rooms

POLICY = 'original-room-route-batch-joint-sampled-completion/1'
MAX_GOAL = original.MAX_GOAL + rooms.MAX_BYTES + 18
MAX_SAMPLE = construction.MAX_SAMPLE + 2 * routes.MAX_CAPTURE_BYTES + 1024


def idle():
    pass


def _blob(raw, maximum):
    require(type(raw) is bytes and 0 < len(raw) <= maximum, 'room completion extent exceeded')
    return struct.pack('>I', len(raw)) + raw


def _read(reader, maximum):
    return reader.take(integer(reader.u32(maximum), 1, maximum))


@dataclass(frozen=True)
class Goal:
    original: original.Goal
    handoff: rooms.RoomRouteHandoff
    require_circulation: bool = False
    require_project_access: bool = False
    generation: int = 1

    def validate(self, guard=idle):
        guard()
        require(type(self.original) is original.Goal and type(self.handoff) is rooms.RoomRouteHandoff,
                'complete original room batch and completion goal required')
        self.original.validate(guard)
        room, review = self.handoff.unpack(guard)
        _, request, source, plan = review.unpack(guard)
        require(type(self.require_circulation) is bool
                and type(self.require_project_access) is bool
                and self.generation in (1, 2),
                'invalid room completion requirement generation')
        if self.generation == 1:
            require(not self.require_circulation and not self.require_project_access,
                    'legacy room completion goal cannot acquire new requirements')
        else:
            intent = room.json()['intent']
            expected_circulation = bool(intent.get('connect_areas', False))
            expected_access = 'access' in intent
            require(self.require_circulation == expected_circulation
                    and self.require_project_access == expected_access
                    and (not self.require_project_access or self.require_circulation),
                    'room completion requirements differ from retained room intent')
        require(review.digest == self.original.review_digest and plan == self.original.plan
                and source.inventory_generation == self.original.operations_generation
                and (source.df_version, source.dfhack_version) == self.original.build.software,
                'room parent changed the original batch construction goal')
        for member in self.condition.goals:
            guard()
            before = member.record.plan.before
            bind_native_selection(review, before, guard)
            require(before.dimensions == source.dimensions and before.tick >= source.tick,
                    'original placement predates or differs from room source')
        guard()

    @property
    def condition(self):
        return self.original.condition

    @property
    def receipts(self):
        return self.original.receipts

    @property
    def batch_id(self):
        return self.original.batch_id

    @property
    def origin(self):
        return self.original.origin

    @property
    def review_digest(self):
        return self.original.review_digest

    @property
    def plan(self):
        return self.original.plan

    @property
    def build(self):
        return self.original.build

    @property
    def operations_generation(self):
        return self.original.operations_generation

    def encode(self, guard=idle):
        self.validate(guard)
        if self.generation == 1:
            prefix = b'DFMRQG01'
        else:
            prefix = b'DFMRQG02' + bytes([self.require_circulation, self.require_project_access])
        raw = (prefix + _blob(self.original.encode(guard), original.MAX_GOAL)
               + _blob(self.handoff.encode(guard), rooms.MAX_BYTES))
        require(len(raw) <= MAX_GOAL, 'complete room goal exceeds bound')
        guard()
        return raw

    @classmethod
    def decode(cls, raw, guard=idle):
        guard()
        reader = Cursor(raw, MAX_GOAL)
        magic = reader.take(8)
        require(magic in (b'DFMRQG01', b'DFMRQG02'),
                'wrong room-route completion generation')
        if magic == b'DFMRQG01':
            circulation, access, generation = False, False, 1
        else:
            circulation = bool(integer(reader.number(1), 0, 1))
            access = bool(integer(reader.number(1), 0, 1))
            generation = 2
        child = original.Goal.decode(_read(reader, original.MAX_GOAL), guard)
        parent = _read(reader, rooms.MAX_BYTES)
        reader.finish()
        value = cls(child, rooms.RoomRouteHandoff.decode(parent, rooms.object_digest(parent), guard),
                    circulation, access, generation)
        require(value.encode(guard) == raw, 'noncanonical complete room goal')
        return value

    @property
    def digest(self):
        return hashlib.sha256(b'dfmcp-room-route-completion-goal/1\0' + self.encode()).hexdigest()


def from_batch(batch, deadline, **timing):
    handoff = getattr(batch, 'room_handoff', None)
    require(type(handoff) is rooms.RoomRouteHandoff, 'room completion requires an original room-backed batch')
    room, _ = handoff.unpack(batch.budget.work)
    intent = room.json()['intent']
    goal = Goal(original.from_batch(batch, deadline, **timing), handoff,
                bool(intent.get('connect_areas', False)), 'access' in intent, 2)
    verify_origin(goal, batch)
    return goal


def verify_origin(goal, batch):
    require(type(goal) is Goal, 'exact room completion goal required')
    goal.validate(batch.budget.work)
    original.verify_origin(goal.original, batch)
    handoff = getattr(batch, 'room_handoff', None)
    require(type(handoff) is rooms.RoomRouteHandoff and handoff.digest == goal.handoff.digest
            and handoff.raw == goal.handoff.raw, 'original room parent changed or disappeared')
    batch.check()


def _occupancy(raw, terrain, plan, guard):
    """Map occupancy corroborates the roster; it never identifies a building."""
    targets = {terrain.index(step.target): step for step in plan.steps}
    require(None not in targets and len(targets) == len(plan.steps), 'room targets outside current map')
    reader = routes.Reader(raw)
    reader.take(21)
    reader.take(reader.number('H'))
    reader.take(40)
    missing = []
    for index in range(len(terrain.shapes)):
        guard()
        presence = reader.number('B')
        attributes = struct.unpack('>IBBBBBBBIHH', reader.take(19)) if presence == 2 else None
        if index in targets and (attributes is None or not attributes[6]):
            step = targets[index]
            missing.append({'slot': step.name, 'target': list(step.target)})
    require(reader.offset == len(raw), 'room occupancy did not consume the validated map')
    return {'all_targets_have_building_occupancy': not missing, 'missing_targets': missing,
            'building_identity_from_map_proven': False}


@dataclass(frozen=True)
class Sample:
    map_before: Manifest
    before_capture: bytes
    furnishings: construction.LinkedSample
    map_after: Manifest
    after_capture: bytes

    def encode(self):
        require(type(self.map_before) is Manifest and type(self.map_after) is Manifest
                and type(self.furnishings) is construction.LinkedSample, 'invalid room sample types')
        raw = (b'DFMRQS01' + self.map_before.encode()
               + _blob(self.before_capture, routes.MAX_CAPTURE_BYTES)
               + _blob(self.furnishings.encode(), construction.MAX_SAMPLE)
               + self.map_after.encode() + _blob(self.after_capture, routes.MAX_CAPTURE_BYTES))
        require(len(raw) <= MAX_SAMPLE, 'complete room sample exceeds bound')
        return raw

    @classmethod
    def decode(cls, raw):
        reader = Cursor(raw, MAX_SAMPLE)
        require(reader.take(8) == b'DFMRQS01', 'wrong room completion sample generation')
        before = Manifest.read(reader)
        capture = _read(reader, routes.MAX_CAPTURE_BYTES)
        child = construction.LinkedSample.decode(_read(reader, construction.MAX_SAMPLE))
        after = Manifest.read(reader)
        value = cls(before, capture, child, after, _read(reader, routes.MAX_CAPTURE_BYTES))
        reader.finish()
        require(value.encode() == raw, 'noncanonical room completion sample')
        return value

    def validate(self, goal, guard=idle):
        guard()
        self.encode()
        room, review = goal.handoff.unpack(guard)
        _, request, source, plan = review.unpack(guard)
        require(self.map_before == self.map_after == Manifest(source.map_generation, *goal.build.software)
                and self.before_capture == self.after_capture,
                'room map endpoints or original map incarnation differ')
        require(self.furnishings.before == self.furnishings.after == goal.build
                and self.furnishings.operations == Manifest(source.inventory_generation, *goal.build.software),
                'room sample changed original build or inventory source')
        observed = self.furnishings.validate(goal.condition, guard)
        terrain = routes.decode_terrain(self.before_capture, guard)
        require((terrain.folder, terrain.site, terrain.dimensions, terrain.origin, terrain.size) ==
                (request.folder, request.site, source.dimensions, source.origin, source.size),
                'room map differs from the complete original region')
        require((observed.folder, observed.site) == (request.folder, request.site)
                and terrain.paused and observed.paused and terrain.tick == observed.tick
                and observed.tick >= source.tick
                and all(a >= b for a, b in zip(observed.horizons, source.horizons)),
                'room sources disagree or predate original intent')
        diagnosis = rooms._terrain_report(room, self.before_capture, guard)
        diagnosis['furniture_occupancy'] = _occupancy(self.before_capture, terrain, plan, guard)
        if goal.require_circulation:
            import room_readiness
            capture = map_wire.decode_capture(
                self.before_capture,
                map_wire.Manifest(self.map_before.generation, *self.map_before.software),
                map_wire.Region(source.origin, source.size),
            )
            proxy = SimpleNamespace(
                room=SimpleNamespace(room_plan=room),
                require_circulation=True,
                require_project_access=goal.require_project_access,
            )
            diagnosis['circulation'] = room_readiness._circulation(proxy, capture, guard)
        guard()
        return terrain, diagnosis

    @property
    def digest(self):
        return hashlib.sha256(b'dfmcp-room-route-completion-sample/1\0' + self.encode()).hexdigest()


@dataclass(frozen=True)
class Progress(construction.Progress):
    map_witness: str | None = None
    joint_sample_digest: str | None = None
    room_diagnosis: bytes | None = None
    circulation_required: bool = False
    project_access_required: bool = False

    def view(self):
        out = super().view()
        out.update(policy=POLICY, map_observation_sha256=self.map_witness,
                   joint_sample_digest=self.joint_sample_digest,
                   room_at_last_observation=None if self.room_diagnosis is None else json.loads(self.room_diagnosis),
                   sampled_whole_room_condition=self.phase == 'satisfied',
                   atomic_cross_profile_snapshot_proven=False, continuous_wall_preservation_proven=False,
                   native_room_assignment_proven=False, current_room_usability_proven=False,
                   circulation_required=self.circulation_required,
                   project_access_required=self.project_access_required,
                   room_completion_proven=False)
        return out


def begin_read(state):
    require(type(state) is Progress, 'exact room progress required')
    return construction.begin_read(state)


def cancel(state):
    require(type(state) is Progress, 'exact room progress required')
    return construction.cancel(state)


def advance(state, goal, sample, guard=idle):
    guard()
    require(type(state) is Progress and type(goal) is Goal and type(sample) is Sample,
            'exact room completion transition required')
    goal.validate(guard)
    require(state.goal_digest == goal.digest and state.reading and not state.terminal,
            'invalid room completion transition')
    terrain, diagnosis = sample.validate(goal, guard)
    witness = sample.digest
    good_room = (diagnosis['all_required_shapes_at_sample']
                 and diagnosis['furniture_occupancy']['all_targets_have_building_occupancy'])
    good_access = (not goal.require_circulation
                   or diagnosis['circulation']['all_required_routes_clear'])
    good = good_room and good_access
    changed = (state.last_tick == terrain.tick and state.joint_sample_digest is not None
               and state.joint_sample_digest != witness)
    previous = replace(state, goal_digest=goal.condition.digest)
    if not good or changed:
        previous = replace(previous, streak=0, first_tick=None, counted_tick=None)
    current = construction.advance(previous, goal.condition, sample.furnishings, guard)
    current = replace(current, goal_digest=goal.digest, map_witness=terrain.digest,
                      joint_sample_digest=witness, room_diagnosis=canonical(diagnosis),
                      circulation_required=goal.require_circulation,
                      project_access_required=goal.require_project_access)
    if current.phase in ('active', 'candidate', 'satisfied') and (not good or changed):
        exhausted = current.observations >= goal.condition.max_observations
        current = replace(current, phase='expired' if exhausted else 'active',
            reason='sample_budget_exhausted' if exhausted else 'original_room_not_ready'
                   if not good_room else 'original_room_access_not_ready'
                   if not good_access else 'room_sample_changed_at_same_tick',
            reason_building=None, streak=0, first_tick=None, counted_tick=None)
    guard()
    return current
