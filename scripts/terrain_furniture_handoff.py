"""Bind exact furnishings to completed terrain custody and raw fresh map evidence.

A new closed artifact, not a legacy handoff with discarded extra metadata.
Consumers must verify the original journal before new effects; recovery of an
old effect must not depend on its availability. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass
import hashlib

import excavation_observer as e
import room_terrain_goal as r
from furniture_allocation import Guard, idle
from furniture_plan import canonical, require
from room_furniture_handoff import RoomFurnitureHandoff
from room_terrain_origin import TerrainOrigin, read_json

SCHEMA = 'dfmcp.terrain-furniture-handoff/1'
MAX_BYTES = 320 * 1024
MAX_DEPTH = 15


def clear_targets(goal: r.RoomTerrainGoal, capture: e.Capture, guard: Guard) -> None:
    diagnosis = r.diagnose(goal, capture, guard=guard)
    require(diagnosis['all_required_shapes_at_sample'] and diagnosis['deficits']['count'] == 0,
            'original room floors and walls no longer hold together')
    ox, oy, oz = goal.region.origin
    sx, sy, _ = goal.region.size
    for slot in goal.room_plan.request().slots:
        guard()
        x, y, z = slot.target
        tile = capture.tiles[((z - oz) * sy + y - oy) * sx + x - ox]
        require(tile.presence == 2, 'original furniture target is not visible')
        _, shape, depth, magma, _, dig, building, units, *_ = tile.attributes
        require(shape == 3 and not (depth or magma or dig or building or units),
                'original furniture target is not clear dry undesignated floor')
    guard()


@dataclass(frozen=True)
class TerrainFurnitureHandoff:
    room: RoomFurnitureHandoff
    origin: TerrainOrigin
    fresh: e.Capture
    checkpoint: InitVar[Guard | None] = None

    def __post_init__(self, checkpoint: Guard | None) -> None:
        guard = idle if checkpoint is None else checkpoint
        guard()
        require(type(self.room) is RoomFurnitureHandoff and type(self.origin) is TerrainOrigin,
                'exact original room allocation and terrain origin required')
        room = RoomFurnitureHandoff.decode(self.room.encode(), guard)
        origin = TerrainOrigin.decode(self.origin._raw, guard)
        goal = origin.goal(room.room_plan, guard)
        require(type(self.fresh) is e.Capture, 'raw fresh terrain capture required')
        fresh = r.decode(goal, self.fresh, guard)
        original = origin.json()
        require(fresh.binding() == original['source'], 'fresh map changed the original terrain source')
        room.allocation.validate_binding(original['endpoint'], fresh.folder, fresh.site,
                                        fresh.manifest.df_version, fresh.manifest.dfhack_version)
        require(original['completed_tick'] <= room.allocation.source.tick <= fresh.tick
                and fresh.tick - original['completed_tick'] <= goal.max_gap_ticks,
                'allocation predates completion, follows map capture or exceeds original freshness gap')
        room.check_dimensions(fresh.dimensions, guard)
        clear_targets(goal, fresh, guard)
        for name, value in (('room', room), ('origin', origin), ('fresh', fresh)):
            object.__setattr__(self, name, value)
        require(len(self.encode()) <= MAX_BYTES, 'complete terrain furniture handoff exceeds bound')
        guard()

    def json(self) -> dict:
        return {'schema': SCHEMA, 'room_handoff': self.room.json(), 'terrain_origin': self.origin.json(),
                'fresh_map': {'manifest': self.fresh.manifest.json(), 'capture_hex': self.fresh.raw.hex()}}

    def encode(self) -> bytes:
        return canonical(self.json())

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-terrain-furniture-handoff/1\0' + self.encode()).hexdigest()

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> TerrainFurnitureHandoff:
        value = read_json(raw, MAX_BYTES, MAX_DEPTH, guard)
        require(set(value) == {'schema', 'room_handoff', 'terrain_origin', 'fresh_map'}
                and value['schema'] == SCHEMA, 'invalid terrain furniture handoff schema')
        room = RoomFurnitureHandoff.from_json(value['room_handoff'], guard)
        origin = TerrainOrigin.decode(canonical(value['terrain_origin']), guard)
        goal = origin.goal(room.room_plan, guard)
        sample = value['fresh_map']
        require(type(sample) is dict and set(sample) == {'manifest', 'capture_hex'}, 'invalid fresh map evidence')
        text = sample['capture_hex']
        require(type(text) is str and 2 <= len(text) <= 2 * r.terrain.MAX_CAPTURE_BYTES
                and len(text) % 2 == 0, 'fresh map hexadecimal exceeds bound')
        data = bytes.fromhex(text)
        require(data.hex() == text, 'noncanonical fresh map hexadecimal')
        capture = e.decode_capture(data, e.Manifest.from_json(sample['manifest']), goal.region)
        result = cls(room, origin, capture, checkpoint=guard)
        require(result.encode() == raw, 'terrain furniture handoff is not canonical')
        guard()
        return result

    @classmethod
    def from_json(cls, value: object, guard: Guard = idle) -> TerrainFurnitureHandoff:
        return cls.decode(canonical(value), guard)

    def bind_placement(self, capture, manifest, address: str) -> None:
        # Map, inventory and placement generation numbers are independent.
        require((address, capture.folder, capture.site, capture.dimensions,
                 manifest.df_version, manifest.dfhack_version) ==
                (self.origin.json()['endpoint'], self.fresh.folder, self.fresh.site, self.fresh.dimensions,
                 self.fresh.manifest.df_version, self.fresh.manifest.dfhack_version)
                and capture.tick >= self.fresh.tick, 'placement changed or predates retained terrain source')

    def compact(self) -> dict:
        return {'schema': SCHEMA, 'terrain_handoff_digest': self.digest,
                'room_handoff_digest': self.room.digest, 'terrain_origin': self.origin.compact(),
                'fresh_map_witness': self.fresh.witness, 'fresh_map_tick': self.fresh.tick,
                'terrain_history_verified_this_call': False, 'current_terrain_proven': False,
                'room_completion_proven': False, 'reallocation_permitted': False}
