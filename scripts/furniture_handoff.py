"""Retained inventory-to-placement intent for df-dfhack-bridge-plane-c-pic.4/.5.

Pure data, never a placement permit or an independently authenticated observation.
The effect owner must acquire the inventory, retain this complete handoff inside
original batch custody, and validate each later native BEFORE capture against it.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import ipaddress
import json
import re

from furniture_allocation import Candidate, Request, distance
from furniture_plan import FurniturePlan, Step, canonical, integer, require, unique

SCHEMA = 'dfmcp.furniture-handoff-python/1'
MAX_BYTES = 32768
MAX_CAPTURE = 16 * 1024 * 1024
MAX_TICK = (2**32 - 1) * 403200 + 403199


def exact(value: object, keys: set[str]) -> dict:
    require(type(value) is dict and set(value) == keys, 'invalid handoff fields')
    return value


def endpoint(value: object) -> str:
    require(type(value) is str and 1 <= len(value) <= 128 and value.count(':') == 1,
            'handoff requires numeric loopback endpoint')
    host, port = value.split(':')
    require(port.isascii() and port.isdecimal() and 1 <= len(port) <= 5,
            'invalid handoff endpoint port')
    address = ipaddress.IPv4Address(host)
    number = integer(int(port), 1, 65535)
    require(address.is_loopback and value == f'{address}:{number}', 'noncanonical handoff endpoint')
    return value


def version(value: object) -> str:
    require(type(value) is str and '\0' not in value and 1 <= len(value.encode('utf-8')) <= 128,
            'invalid handoff software version')
    return value


@dataclass(frozen=True)
class InventorySource:
    address: str
    generation: int
    df_version: str
    dfhack_version: str
    capture_sha256: str
    capture_bytes: int
    tick: int
    horizons: tuple[int, int, int]  # Native jobs, buildings, items, in wire order.

    def __post_init__(self) -> None:
        endpoint(self.address)
        integer(self.generation, 1, 2**64 - 2)
        version(self.df_version)
        version(self.dfhack_version)
        require(type(self.capture_sha256) is str
                and re.fullmatch('[0-9a-f]{64}', self.capture_sha256) is not None,
                'invalid inventory capture digest')
        integer(self.capture_bytes, 1, MAX_CAPTURE)
        integer(self.tick, 0, MAX_TICK)
        require(type(self.horizons) is tuple and len(self.horizons) == 3,
                'invalid native identity horizons')
        for value in self.horizons:
            integer(value, 0, 2**31 - 1)

    def json(self) -> dict:
        return {'profile': 'operations/1.4', 'endpoint': self.address,
                'generation': self.generation, 'df_version': self.df_version,
                'dfhack_version': self.dfhack_version, 'capture_sha256': self.capture_sha256,
                'capture_bytes': self.capture_bytes, 'game_tick': self.tick,
                'native_horizons': list(self.horizons)}

    @classmethod
    def from_json(cls, value: object) -> InventorySource:
        value = exact(value, {'profile', 'endpoint', 'generation', 'df_version', 'dfhack_version',
                             'capture_sha256', 'capture_bytes', 'game_tick', 'native_horizons'})
        require(value['profile'] == 'operations/1.4' and type(value['native_horizons']) is list,
                'wrong handoff inventory profile or horizons')
        return cls(value['endpoint'], value['generation'], value['df_version'], value['dfhack_version'],
                   value['capture_sha256'], value['capture_bytes'], value['game_tick'],
                   tuple(value['native_horizons']))


@dataclass(frozen=True)
class Selected:
    slot: str
    candidate: Candidate
    native_type: int

    def json(self) -> dict:
        return {'slot': self.slot, 'candidate': self.candidate.json(), 'native_type': self.native_type}

    @classmethod
    def from_json(cls, value: object) -> Selected:
        value = exact(value, {'slot', 'candidate', 'native_type'})
        c = exact(value['candidate'], {'item', 'kind', 'position', 'material', 'subtype'})
        require(type(c['position']) is list and type(c['material']) is list,
                'invalid selected item coordinate/material arrays')
        return cls(value['slot'], Candidate(c['item'], c['kind'], tuple(c['position']),
                                           tuple(c['material']), c['subtype']), value['native_type'])


@dataclass(frozen=True)
class Handoff:
    request: Request
    source: InventorySource
    selections: tuple[Selected, ...]

    def __post_init__(self) -> None:
        require(type(self.request) is Request and type(self.source) is InventorySource,
                'invalid handoff request/source')
        self.request.__post_init__()
        self.source.__post_init__()
        require(type(self.selections) is tuple and len(self.selections) == len(self.request.slots)
                and all(type(row) is Selected for row in self.selections),
                'handoff must retain every requested slot')
        slots = {s.name: s for s in self.request.slots}
        seen, items, native_types, kinds = set(), set(), {}, {}
        for row in self.selections:
            require(type(row.slot) is str and row.slot in slots and row.slot not in seen
                    and type(row.candidate) is Candidate, 'invalid or duplicate handoff selection')
            row.candidate.__post_init__()
            integer(row.native_type, 0, 2**31 - 1)
            item = row.candidate
            require(item.id not in items and item.id not in self.request.excluded_items
                    and item.id < self.source.horizons[2], 'excluded, reused or out-of-horizon item')
            require(distance(slots[row.slot], item) is not None, 'selection violates original request')
            require(native_types.get(row.native_type, item.kind) == item.kind
                    and kinds.get(item.kind, row.native_type) == row.native_type,
                    'inconsistent selected native type identity')
            native_types[row.native_type], kinds[item.kind] = item.kind, row.native_type
            seen.add(row.slot)
            items.add(item.id)
        object.__setattr__(self, 'selections', tuple(sorted(self.selections, key=lambda row: row.slot)))
        self.plan()  # Validate the exact executable artifact, not only candidate counts.
        require(len(canonical(self.json())) <= MAX_BYTES, 'complete handoff exceeds 32 KiB')

    def plan(self) -> FurniturePlan:
        by_slot = {row.slot: row.candidate.id for row in self.selections}
        return FurniturePlan(tuple(Step(s.name, s.kind, by_slot[s.name], s.target, s.after)
                                   for s in self.request.slots))

    def json(self) -> dict:
        return {'schema': SCHEMA, 'request': self.request.json(), 'inventory': self.source.json(),
                'selections': [row.json() for row in self.selections]}

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-furniture-handoff-python/1\0' + canonical(self.json())).hexdigest()

    def compact(self) -> dict:
        return {'schema': SCHEMA, 'handoff_digest': self.digest, 'request_digest': self.request.digest,
                'capture_sha256': self.source.capture_sha256, 'inventory_game_tick': self.source.tick,
                'constraints_retained': True, 'items_reserved': False, 'reallocation_permitted': False,
                'native_acquisition_independently_attested': False}

    @classmethod
    def from_json(cls, value: object) -> Handoff:
        value = exact(value, {'schema', 'request', 'inventory', 'selections'})
        require(value['schema'] == SCHEMA and type(value['selections']) is list
                and 1 <= len(value['selections']) <= 32, 'invalid complete handoff')
        return cls(Request.from_json(value['request']), InventorySource.from_json(value['inventory']),
                   tuple(Selected.from_json(row) for row in value['selections']))

    @classmethod
    def decode(cls, raw: bytes) -> Handoff:
        require(type(raw) is bytes and 1 <= len(raw) <= MAX_BYTES, 'handoff byte bound exceeded')
        # Bound nesting before invoking the general JSON parser, including hostile strings.
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
                require(depth <= 10, 'handoff nesting bound exceeded')
            elif byte in (93, 125):
                depth -= 1
        value = cls.from_json(json.loads(raw.decode('utf-8'), object_pairs_hook=unique))
        require(canonical(value.json()) == raw, 'handoff must be complete canonical JSON')
        return value

    def validate_binding(self, address: str, folder: str, site: int,
                         df_version: str, dfhack_version: str) -> None:
        require((endpoint(address), folder, integer(site, 0, 2**31 - 1),
                 version(df_version), version(dfhack_version)) ==
                (self.source.address, self.request.folder, self.request.site,
                 self.source.df_version, self.source.dfhack_version),
                'placement does not match original inventory source')
        # Operations generation and furniture generation are deliberately NOT compared.
        # The batch independently binds the furniture generation on every read/replay.

    def validate_item(self, step: Step, candidate: Candidate, native_type: int,
                      tick: int, next_job: int, next_building: int) -> None:
        """Check a decoded native BEFORE capture, never a post-placement item.

        Position may change within the original distance/level constraint. Identity,
        native type, material and subtype may not. This method only adds refusals;
        it does not establish native eligibility or issue a connection permit.
        """
        require(type(step) is Step and type(candidate) is Candidate, 'invalid placement selection')
        candidate.__post_init__()
        integer(native_type, 0, 2**31 - 1)
        integer(tick, 0, MAX_TICK)
        integer(next_job, 0, 2**31 - 1)
        integer(next_building, 0, 2**31 - 1)
        require(tick >= self.source.tick and next_job >= self.source.horizons[0]
                and next_building >= self.source.horizons[1], 'placement predates original inventory')
        original = next((row for row in self.selections if row.slot == step.name), None)
        require(original is not None and step in self.plan().steps, 'placement changed original plan')
        item = original.candidate
        require((candidate.id, candidate.kind, candidate.material, candidate.subtype, native_type) ==
                (item.id, item.kind, item.material, item.subtype, original.native_type),
                'selected item changed since allocation')
        slot = next(s for s in self.request.slots if s.name == step.name)
        require(distance(slot, candidate) is not None, 'current item violates retained distance or level')
