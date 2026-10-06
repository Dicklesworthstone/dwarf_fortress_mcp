"""Keep complete room intent attached to an exact furniture allocation.

This is historical intent and inventory evidence, not terrain-completion proof,
item reservation, native acquisition attestation or permission to place anything.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10; remain open).
"""
from __future__ import annotations

from dataclasses import InitVar, dataclass
import hashlib
import json

from furniture_allocation import Guard, idle
from furniture_handoff import Handoff
from furniture_plan import canonical, integer, require, unique
from room_provisioning import MAX_PLAN_BYTES, RoomPlan

SCHEMA = 'dfmcp.room-furniture-handoff/1'
MAX_BYTES = 96 * 1024
MAX_DEPTH = 13


def _read_json(raw: bytes, guard: Guard) -> dict:
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_BYTES,
            'room furniture handoff exceeds its byte bound')
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
            require(depth <= MAX_DEPTH, 'room furniture handoff nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1
    def nonfinite(_value: str):
        raise ValueError('nonfinite room furniture handoff number')
    value = json.loads(raw.decode('utf-8'), object_pairs_hook=unique, parse_constant=nonfinite)
    require(type(value) is dict and set(value) == {'schema', 'room_plan', 'allocation'}
            and value['schema'] == SCHEMA, 'invalid room furniture handoff envelope')
    guard()
    return value


@dataclass(frozen=True)
class RoomFurnitureHandoff:
    room_plan: RoomPlan
    allocation: Handoff
    checkpoint: InitVar[Guard | None] = None

    def __post_init__(self, checkpoint: Guard | None) -> None:
        guard = idle if checkpoint is None else checkpoint
        guard()
        require(type(self.room_plan) is RoomPlan and type(self.allocation) is Handoff,
                'complete original room plan and inventory allocation required')
        # Do not trust directly constructed dataclasses or cached derived fields.
        require(type(self.room_plan._body) is bytes
                and 1 <= len(self.room_plan._body) <= MAX_PLAN_BYTES,
                'invalid original room plan extent')
        room = RoomPlan.decode(self.room_plan.encode(), guard)
        self.allocation.__post_init__()
        allocation = Handoff.decode(canonical(self.allocation.json()))
        require(canonical(room.request().json()) == canonical(allocation.request.json()),
                'allocation changed or omitted original room furnishing constraints')
        object.__setattr__(self, 'room_plan', room)
        object.__setattr__(self, 'allocation', allocation)
        require(len(self.encode()) <= MAX_BYTES, 'complete room allocation exceeds handoff bound')
        guard()

    def json(self) -> dict:
        return {'schema': SCHEMA, 'room_plan': self.room_plan.json(),
                'allocation': self.allocation.json()}

    def encode(self) -> bytes:
        return canonical(self.json())

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-room-furniture-handoff/1\0' + self.encode()).hexdigest()

    @classmethod
    def decode(cls, raw: bytes, guard: Guard = idle) -> RoomFurnitureHandoff:
        value = _read_json(raw, guard)
        room = RoomPlan.decode(canonical(value['room_plan']), guard)
        allocation = Handoff.decode(canonical(value['allocation']))
        result = cls(room, allocation, checkpoint=guard)
        require(result.encode() == raw, 'room furniture handoff must be complete canonical JSON')
        guard()
        return result

    @classmethod
    def from_json(cls, value: object, guard: Guard = idle) -> RoomFurnitureHandoff:
        # File/transport owners bound their containing envelope before this call.
        guard()
        return cls.decode(canonical(value), guard)

    def check_dimensions(self, dimensions: object, guard: Guard = idle) -> None:
        """Fit the ENTIRE original floor/corridor and required wall geometry.

        Fitting the furniture targets alone can hide an out-of-map room wall or
        corridor. These are dimension checks only, never observations of terrain.
        """
        require(type(dimensions) in (tuple, list) and len(dimensions) == 3,
                'three native map dimensions required')
        for size in dimensions:
            integer(size, 1, 32768)
        self.allocation.plan().check_dimensions(dimensions)
        value = self.room_plan.json()
        region = value['capture_region']
        require(all(0 <= start and start + size <= bound for start, size, bound in
                    zip(region['origin'], region['size'], dimensions)),
                'original room floor or corridor lies outside native map')
        kinds = {area['name']: area['template']['kind'] for area in value['intent']['areas']}
        for area in value['areas']:
            guard()
            if kinds[area['name']] != 'bedroom_cluster':
                continue
            for unit in area['units']:
                guard()
                x, y, z = unit['origin']
                width, height, _ = unit['size']
                require(0 <= x - 1 and x + width < dimensions[0]
                        and 0 <= y - 1 and y + height < dimensions[1]
                        and 0 <= z < dimensions[2],
                        'original required bedroom wall lies outside native map')
        guard()

    def compact(self) -> dict:
        return {'schema': SCHEMA, 'room_handoff_digest': self.digest,
                'room_plan_digest': self.room_plan.digest,
                'allocation_digest': self.allocation.digest,
                'furniture_request_digest': self.allocation.request.digest,
                'complete_original_room_intent_retained': True,
                'reallocation_permitted': False, 'items_reserved': False,
                'terrain_completion_proven': False, 'room_completion_proven': False,
                'native_acquisition_independently_attested': False}
