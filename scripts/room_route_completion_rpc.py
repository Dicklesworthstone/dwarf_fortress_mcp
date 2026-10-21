"""One query-only receipt/map/inventory bracket for original room completion.

Reuse the existing construction transport and room-map methods. No placement,
clock effect, extra source, reconnect or failed-read retry is introduced.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from dataclasses import dataclass

from furniture_plan import integer, require
import construction_plan_rpc as construction
import construction_monitor_rpc as wire
from construction_plan import LinkedSample
from construction_receipt import Manifest
import room_readiness_rpc as room_wire
from room_route_completion import Goal, Sample

Authority = room_wire.Authority
Budget = room_wire.Budget
MAX_RPC_CALLS = room_wire.MAX_RPC_CALLS


@dataclass(frozen=True)
class _Region:
    origin: tuple[int, int, int]
    size: tuple[int, int, int]

    @property
    def volume(self):
        return self.size[0] * self.size[1] * self.size[2]


class Client(construction.Client):
    # These source-selected methods use the exact bounded region below; no
    # caller chooses a method name, profile, decoder or credential family.
    _map_frame = room_wire.Client._map_frame
    _map_call = room_wire.Client._map_call

    def __init__(self, authority, goal, budget):
        require(type(goal) is Goal and type(authority) is Authority,
                'exact original room goal and read authority required')
        def guard():
            budget.work()
            authority.guard()
        goal = Goal.decode(goal.encode(guard), guard)
        _, review = goal.handoff.unpack(guard)
        source = review.unpack(guard)[2]
        require(authority.address == wire.endpoint(source.endpoint),
                'room monitor endpoint differs from original source')
        self.room_goal = goal
        self.region = _Region(source.origin, source.size)
        self.map_manifest, self.map_reads = None, 0
        super().__init__(authority, goal.condition, budget)
        try:
            require(self.manifests['build'] == goal.build
                    and self.manifests['operations'] == Manifest(source.inventory_generation, *goal.build.software),
                    'original room construction sources changed at handshake')
            for name in room_wire.MAP_METHODS:
                guard()
                reply = wire.decode(self._frame(0, wire.encode({1: name.encode('ascii'),
                    2: b'dfmcp.map.v1_5.Request', 3: b'dfmcp.map.v1_5.Reply',
                    4: b'dfmcp_map_v1_5'})), 1)
                require(set(reply) == {1}, 'invalid room completion map binding')
                method = integer(reply[1], 2, 32767)
                require(method not in self.methods.values(), 'aliased room completion method binding')
                self.methods['map', name] = method
            self._map_call(False)
            require(self.map_manifest == Manifest(source.map_generation, *goal.build.software),
                    'original room map incarnation changed at handshake')
        except BaseException:
            self.close()
            raise

    def capture_once(self):
        require(not self.used and not self.closed, 'room completion acquisition already consumed')
        self.used = True
        try:
            records_before = tuple(self._member_receipt(goal) for goal in self.member_goals)
            build_before = self.manifests['build']
            map_before = self._map_call(True)
            manifest_before = self.map_manifest
            operations_raw = self._capture_operations()
            map_after = self._map_call(True)
            records_after = tuple(self._member_receipt(goal) for goal in self.member_goals)
            sample = Sample(manifest_before, map_before,
                LinkedSample(build_before, records_before, self.manifests['operations'], operations_raw,
                             self.manifests['build'], records_after), self.map_manifest, map_after)
            self.close()
            def guard():
                self.budget.work()
                self.authority.guard()
            sample.validate(self.room_goal, guard)
            guard()
            return sample
        except BaseException:
            self.close()
            raise


def acquire(authority, goal, budget):
    with Client(authority, goal, budget) as client:
        return client.capture_once()
