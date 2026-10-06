"""One sampled condition for original room terrain AND installed furnishings.

The map brackets the complete receipt-linked operations capture. Matching paused
endpoint samples are not an atomic snapshot, continuous safety, native room
assignment, or effect authority. The durable owner must bind original batch
custody separately. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-05/07/10.
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass, replace
import hashlib
import json
import struct

import construction_plan as construction
from construction_receipt import Cursor, Manifest
import excavation_observer as map_wire
from furniture_allocation import Candidate, Guard, idle
from furniture_plan import canonical, require
from room_furniture_handoff import RoomFurnitureHandoff, MAX_BYTES as MAX_HANDOFF
import room_terrain_goal as terrain

POLICY = 'dfmcp.original-room-terrain-and-furnishings/1'
MAX_GOAL = MAX_HANDOFF + construction.MAX_GOAL + 16
MAX_SAMPLE = construction.MAX_SAMPLE + 2 * terrain.terrain.MAX_CAPTURE_BYTES + 1024


def _blob(raw: bytes, maximum: int) -> bytes:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum, 'room readiness extent exceeded')
    return struct.pack('>I', len(raw)) + raw


def _read_blob(reader: Cursor, maximum: int) -> bytes:
    return reader.take(map_wire.integer(reader.u32(maximum), 1, maximum))


@dataclass(frozen=True)
class Goal:
    room: RoomFurnitureHandoff
    condition: construction.Goal
    checkpoint: InitVar[Guard | None] = None

    def __post_init__(self, checkpoint: Guard | None) -> None:
        guard = idle if checkpoint is None else checkpoint
        guard()
        require(type(self.room) is RoomFurnitureHandoff and type(self.condition) is construction.Goal,
                'complete original room and construction condition required')
        room = RoomFurnitureHandoff.decode(self.room.encode(), guard)
        condition = construction.Goal.decode(self.condition.encode())
        plan = room.allocation.plan()
        require(len(condition.goals) == len(plan.steps), 'room readiness cannot select a receipt subset')
        steps = {step.item: step for step in plan.steps}
        for child in condition.goals:
            guard()
            record = child.record
            before, selected = record.plan.before, record.plan.before.selection
            step = steps.get(selected.item)
            require(step is not None and selected.target == step.target
                    and ('', 'bed', 'chair', 'table')[selected.kind] == step.kind,
                    'receipt substituted an original room furnishing')
            item = before.item
            room.allocation.validate_item(step, Candidate(selected.item, step.kind, item.pos,
                (item.material, item.material_index), item.subtype), item.native_type,
                before.tick, before.next_job, before.next_building)
            require((before.folder, before.site) == (room.allocation.request.folder, room.allocation.request.site),
                    'receipt belongs to another original fortress')
            room.check_dimensions(before.dimensions, guard)
        # The same complete floor/corridor/wall mask as the existing room monitor.
        # Its temporal reducer is NOT run or latched independently.
        terrain.RoomTerrainGoal(room.room_plan, condition.deadline, checkpoint=guard)
        object.__setattr__(self, 'room', room)
        object.__setattr__(self, 'condition', condition)
        guard()

    def terrain_goal(self, guard: Guard = idle) -> terrain.RoomTerrainGoal:
        return terrain.RoomTerrainGoal(self.room.room_plan, self.condition.deadline, checkpoint=guard)

    def encode(self) -> bytes:
        return (b'DFMRDG01' + _blob(self.room.encode(), MAX_HANDOFF)
                + _blob(self.condition.encode(), construction.MAX_GOAL))

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> Goal:
        reader = Cursor(raw, MAX_GOAL)
        require(reader.take(8) == b'DFMRDG01', 'wrong room readiness goal generation')
        room = RoomFurnitureHandoff.decode(_read_blob(reader, MAX_HANDOFF), guard)
        condition = construction.Goal.decode(_read_blob(reader, construction.MAX_GOAL))
        reader.finish()
        result = cls(room, condition, checkpoint=guard)
        require(result.encode() == raw, 'noncanonical room readiness goal')
        return result

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-readiness-goal/1\0' + self.encode()).hexdigest()


@dataclass(frozen=True)
class Sample:
    map_before: Manifest
    before_capture: bytes
    furnishings: construction.LinkedSample
    map_after: Manifest
    after_capture: bytes

    def encode(self) -> bytes:
        require(type(self.map_before) is Manifest and type(self.map_after) is Manifest
                and type(self.furnishings) is construction.LinkedSample, 'invalid room sample fields')
        raw = (b'DFMRDS01' + self.map_before.encode()
               + _blob(self.before_capture, terrain.terrain.MAX_CAPTURE_BYTES)
               + _blob(self.furnishings.encode(), construction.MAX_SAMPLE)
               + self.map_after.encode() + _blob(self.after_capture, terrain.terrain.MAX_CAPTURE_BYTES))
        require(len(raw) <= MAX_SAMPLE, 'complete room sample too large')
        return raw

    @classmethod
    def decode(cls, raw: bytes) -> Sample:
        reader = Cursor(raw, MAX_SAMPLE)
        require(reader.take(8) == b'DFMRDS01', 'wrong room readiness sample generation')
        before = Manifest.read(reader)
        capture = _read_blob(reader, terrain.terrain.MAX_CAPTURE_BYTES)
        furnishings = construction.LinkedSample.decode(_read_blob(reader, construction.MAX_SAMPLE))
        after = Manifest.read(reader)
        result = cls(before, capture, furnishings, after, _read_blob(reader, terrain.terrain.MAX_CAPTURE_BYTES))
        reader.finish()
        require(result.encode() == raw, 'noncanonical room readiness sample')
        return result

    def validate(self, goal: Goal, guard: Guard) -> tuple[map_wire.Capture, dict]:
        guard()
        self.encode()
        require(self.map_before == self.map_after and self.before_capture == self.after_capture,
                'map evidence changed across the construction capture')
        source = goal.room.allocation.source
        require(self.furnishings.operations == Manifest(source.generation, source.df_version, source.dfhack_version)
                and self.map_before.software == self.furnishings.operations.software,
                'room sample differs from original allocation software or operations incarnation')
        # Generations belong to distinct profiles; they are never equated.
        observed = self.furnishings.validate(goal.condition, guard)
        tg = goal.terrain_goal(guard)
        manifest = map_wire.Manifest(self.map_before.generation, *self.map_before.software)
        capture = map_wire.decode_capture(self.before_capture, manifest, tg.region)
        require((capture.folder, capture.site, capture.dimensions) == (
            goal.room.allocation.request.folder, goal.room.allocation.request.site,
            goal.condition.goals[0].record.plan.before.dimensions), 'room map source differs from original placement')
        require(capture.paused and observed.paused and capture.tick == observed.tick,
                'room sample requires matching paused game ticks across both profiles')
        require(observed.tick >= source.tick and all(a >= b for a, b in zip(observed.horizons, source.horizons)),
                'room sample predates original allocation')
        diagnosis = terrain.diagnose(tg, capture, guard=guard)
        # A completed operations building must not be combined with a map that
        # explicitly shows no building occupancy at its original target. The
        # native occupancy tag does NOT identify which building occupies it;
        # exact identity still comes from the original receipt-linked roster.
        ox, oy, oz = capture.region.origin
        sx, sy, _ = capture.region.size
        missing = []
        for step in goal.room.allocation.plan().steps:
            guard()
            x, y, z = step.target
            tile = capture.tiles[((z - oz) * sy + y - oy) * sx + x - ox]
            if tile.presence != 2 or not tile.attributes[6]:
                missing.append({'step': step.name, 'target': list(step.target)})
        diagnosis['furniture_occupancy'] = {
            'all_targets_have_building_occupancy': not missing,
            'missing_targets': missing, 'building_identity_from_map_proven': False}
        guard()
        return capture, diagnosis

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-readiness-sample/1\0' + self.encode()).hexdigest()


@dataclass(frozen=True)
class Progress(construction.Progress):
    map_source: bytes | None = None
    map_witness: str | None = None
    sample_digest: str | None = None
    terrain_diagnosis: bytes | None = None

    def view(self) -> dict:
        result = super().view()
        result.update(policy=POLICY,
            terrain_at_last_observation=None if self.terrain_diagnosis is None else json.loads(self.terrain_diagnosis),
            map_observation_witness=self.map_witness, joint_sample_digest=self.sample_digest,
            room_readiness_sampled_condition=self.phase == 'satisfied',
            furnishing_conditions_at_sample=bool(self.assessments) and all(
                row.condition.status == 'condition_met' for row in self.assessments),
            atomic_cross_profile_snapshot_proven=False, continuous_wall_preservation_proven=False,
            room_assignments_observed=False, room_completion_proven=False, production_admitted=False)
        return result


def begin_read(state: Progress) -> Progress:
    require(type(state) is Progress, 'exact room readiness progress required')
    return construction.begin_read(state)


def cancel(state: Progress) -> Progress:
    require(type(state) is Progress, 'exact room readiness progress required')
    return construction.cancel(state)


def advance(state: Progress, goal: Goal, sample: Sample, guard: Guard = idle) -> Progress:
    guard()
    require(type(state) is Progress and type(goal) is Goal and type(sample) is Sample,
            'exact room readiness transition inputs required')
    goal = Goal.decode(goal.encode(), guard)
    require(state.goal_digest == goal.digest and state.reading and not state.terminal,
            'invalid room readiness transition')
    captured, diagnosis = sample.validate(goal, guard)
    source = canonical(captured.binding())
    witness = sample.digest
    good_terrain = (diagnosis['all_required_shapes_at_sample']
                    and diagnosis['furniture_occupancy']['all_targets_have_building_occupancy'])
    same_tick_change = (state.last_tick == captured.tick and state.sample_digest is not None
                        and state.sample_digest != witness)
    previous = replace(state, goal_digest=goal.condition.digest)
    if not good_terrain or same_tick_change:
        previous = replace(previous, streak=0, first_tick=None, counted_tick=None)
    # Only this reducer owns temporal progress. Terrain cannot latch a separate
    # terminal success, and a false terrain sample cannot advance its streak.
    current = construction.advance(previous, goal.condition, sample.furnishings, guard)
    current = replace(current, goal_digest=goal.digest, map_source=source,
                      map_witness=captured.witness, sample_digest=witness,
                      terrain_diagnosis=canonical(diagnosis))
    if state.map_source is not None and state.map_source != source:
        current = replace(current, phase='invalidated', reason='room_map_source_changed',
                          reason_building=None, streak=0, first_tick=None, counted_tick=None)
    elif not current.terminal and (not good_terrain or same_tick_change):
        current = replace(current, phase='active', reason=(
            'original_room_terrain_not_ready' if not good_terrain else 'room_sample_changed_at_same_tick'),
            reason_building=None, streak=0, first_tick=None, counted_tick=None)
    guard()
    return current
