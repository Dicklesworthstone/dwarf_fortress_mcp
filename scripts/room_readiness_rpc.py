"""Read original room terrain and furnishings on one query-only connection.

All placement receipts bracket two equal paused map reads surrounding one
released operations capture. This samples endpoints; it cannot prove an atomic
snapshot or continuous stability. No native mutation method is bound.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-07/10.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import os
import struct

import construction_monitor_rpc as wire
import construction_plan_rpc as construction
from construction_plan import LinkedSample
from construction_receipt import Manifest
import excavation_observer as map_wire
from furniture_plan import integer, require
from room_readiness import Goal, Sample

OPT_IN = 'DFMCP_ALLOW_UNADMITTED_ROOM_READINESS'
ENDPOINT = 'DFMCP_ROOM_READINESS_ENDPOINT'
MAP_TOKEN = 'DFMCP_MAP_TOKEN'
ENVIRONMENT = (OPT_IN, ENDPOINT, MAP_TOKEN, wire.BUILD_TOKEN, wire.OPERATIONS_TOKEN)
MAP_METHODS = ('Handshake', 'ReadObservation')
MAX_RPC_CALLS = construction.MAX_RPC_CALLS + 5  # two binds, handshake, two map reads


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    build_token: bytes = field(repr=False)
    operations_token: bytes = field(repr=False)
    map_token: bytes = field(repr=False)

    @classmethod
    def load(cls) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'room readiness requires its isolated query-only environment')
        require(os.environ.get(OPT_IN) == '1', 'explicit room readiness development opt-in required')
        tokens = tuple(os.environ.get(key, '').encode('utf-8')
                       for key in (wire.BUILD_TOKEN, wire.OPERATIONS_TOKEN, MAP_TOKEN))
        require(all(32 <= len(token) <= 256 and b'\0' not in token for token in tokens),
                'invalid room readiness query credential')
        return cls(wire.endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), *tokens)

    def guard(self) -> None:
        require(Authority.load() == self, 'room readiness query authority changed')


class Budget(construction.Budget):
    def __init__(self, timeout_ms: int):
        super().__init__(timeout_ms)
        self.calls = MAX_RPC_CALLS


class Client(construction.Client):
    def __init__(self, authority: Authority, goal: Goal, budget: Budget):
        require(type(goal) is Goal and type(authority) is Authority, 'exact readiness goal and authority required')
        def guard():
            budget.work()
            authority.guard()
        goal = Goal.decode(goal.encode(), guard)
        expected = goal.room.allocation.source
        require(authority.address == wire.endpoint(expected.address), 'readiness endpoint differs from original allocation')
        self.readiness_goal = goal
        self.region = goal.terrain_goal(guard).region
        self.map_manifest = None
        self.map_reads = 0
        super().__init__(authority, goal.condition, budget)
        try:
            require(self.manifests['operations'] == Manifest(expected.generation, expected.df_version, expected.dfhack_version),
                    'operations handshake differs from original inventory')
            for name in MAP_METHODS:
                guard()
                bound = wire.decode(self._frame(0, wire.encode({1: name.encode('ascii'),
                    2: b'dfmcp.map.v1_5.Request', 3: b'dfmcp.map.v1_5.Reply',
                    4: b'dfmcp_map_v1_5'})), 1)
                require(set(bound) == {1}, 'invalid map read binding')
                method = integer(bound[1], 2, 32767)
                require(method not in self.methods.values(), 'aliased cross-profile method binding')
                self.methods['map', name] = method
            self._map_call(False)
        except BaseException:
            self.close()
            raise

    def _map_frame(self, name: str, payload: bytes) -> bytes:
        # Map captures can exceed one operations page. Only these two fixed map
        # method IDs get the exact region-derived limit; other frames keep their
        # unchanged parent limits. Notifications share the parent allowance.
        require(not self.closed and name in MAP_METHODS and len(payload) <= 2048,
                'closed or invalid readiness map request')
        self.authority.guard()
        self.budget.reserve('calls', 1)
        self._send(struct.pack('<h2xi', self.methods['map', name], len(payload)) + payload)
        count, notified = 0, 0
        maximum = 575 + 20 * self.region.volume + 1024
        while True:
            self.authority.guard()
            kind, size = struct.unpack('<h2xi', self._read(8))
            require(kind in (-1, -3) and 0 <= size <= (maximum if kind == -1 else 65536),
                    'invalid readiness map frame bound')
            if kind == -3:
                count += 1
                notified += size
                require(count <= 8 and notified <= 262144 and size <= self.notifications_left,
                        'readiness map notification allowance exhausted')
                self.notifications_left -= size
            raw = self._read(size)
            if kind == -1:
                return raw

    def _map_call(self, observe: bool) -> bytes | None:
        try:
            require(type(observe) is bool and (not observe or self.map_reads < 2),
                    'room map read allowance exhausted')
            if observe:
                self.map_reads += 1
            name = 'ReadObservation' if observe else 'Handshake'
            fields = {1: self.authority.map_token, 2: self.nonce, 3: 1, 4: 5,
                **dict(zip(range(5, 11), self.region.origin + self.region.size)),
                11: max(1024, 575 + 20 * self.region.volume)}
            reply = map_wire.decode(self._map_frame(name, map_wire.encode(fields)))
            require(set(reply) == set(range(1, 10 if observe else 9)), 'unexpected readiness map reply fields')
            for n in (1, 2, 4, 5, 6):
                require(type(reply[n]) is int, 'invalid readiness map scalar')
            for n in (3, 7, 8):
                require(type(reply[n]) is bytes, 'invalid readiness map text')
            require(reply[1] == 1 and reply[2] == 0 and reply[3] == self.nonce
                    and reply[4] == 1 and reply[5] == 5, 'map refusal or nonce/profile mismatch')
            manifest = Manifest(reply[6], wire.text(reply[7], 128), wire.text(reply[8], 128))
            manifest.encode()
            require(self.map_manifest is None or self.map_manifest == manifest, 'map incarnation changed during readiness read')
            require(manifest.software == self.manifests['operations'].software, 'map and operations software disagree')
            raw = reply.get(9)
            if observe:
                require(type(raw) is bytes and 1 <= len(raw) <= fields[11], 'invalid complete room map bytes')
            self.map_manifest = manifest
            self.authority.guard()
            self.budget.work()
            return raw
        except BaseException:
            self.close()
            raise

    def capture_once(self) -> Sample:
        require(not self.used and not self.closed, 'room acquisition already consumed or closed')
        self.used = True
        try:
            records_before = tuple(self._member_receipt(goal) for goal in self.member_goals)
            build_before = self.manifests['build']
            map_before = self._map_call(True)
            manifest_before = self.map_manifest
            raw = self._capture_operations()  # complete retained pages and verified release
            map_after = self._map_call(True)
            records_after = tuple(self._member_receipt(goal) for goal in self.member_goals)
            result = Sample(manifest_before, map_before,
                LinkedSample(build_before, records_before, self.manifests['operations'], raw,
                             self.manifests['build'], records_after), self.map_manifest, map_after)
            self.close()  # CPU-bound decoding must not retain a native connection.
            def guard():
                self.budget.work()
                self.authority.guard()
            result.validate(self.readiness_goal, guard)
            guard()
            return result
        except BaseException:
            self.close()
            raise


def acquire(authority: Authority, goal: Goal, budget: Budget) -> Sample:
    with Client(authority, goal, budget) as client:
        return client.capture_once()
