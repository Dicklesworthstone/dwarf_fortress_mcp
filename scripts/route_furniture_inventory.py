"""Query-only route-aware furniture proposals from a bracketed native capture.

One connection, two unchanged map/1.5 reads around one released operations/1.4
roster. Equal paused endpoints are not an atomic snapshot or continuous proof.
No placement methods, credentials, retry, journal or background work are created.
Beads: df-franken-storage-mvcc-54h.4; df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import hashlib
import os
import secrets
import socket
import struct

import construction_monitor_rpc as wire
from construction_receipt import Manifest, decode_operations
from furniture_allocation import Candidate, Request
from furniture_plan import canonical, integer, require
import route_furniture_allocation as routes
from route_furniture_sequence import allocate_sequenced_routes
import route_furniture_optimal as optimal

OPT_IN = 'DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE'
ENDPOINT = 'DFMCP_ROUTE_FURNITURE_ENDPOINT'
MAP_TOKEN = 'DFMCP_MAP_TOKEN'
ENVIRONMENT = (OPT_IN, ENDPOINT, MAP_TOKEN, wire.OPERATIONS_TOKEN)
BINDINGS = (('operations', 'Handshake'), ('operations', 'ReadObservation'),
            ('map', 'Handshake'), ('map', 'ReadObservation'))
PROFILES = {'operations': wire.PROFILES['operations'],
            'map': ('dfmcp_map_v1_5', 'dfmcp.map.v1_5', 5)}
CANDIDATE_POLICY = 'dfmcp.furniture-inventory-candidates/1'
ACQUISITION_POLICY = 'equal-paused-map-endpoints-around-released-inventory/1'
MAX_OUTPUT = 96 * 1024
MINIMUM_CALLS = 10  # Four bindings, two handshakes, two maps, one page, release.


@dataclass(frozen=True)
class Region:
    origin: tuple[int, int, int]
    size: tuple[int, int, int]

    def __post_init__(self) -> None:
        require(type(self.origin) is tuple and type(self.size) is tuple
                and len(self.origin) == len(self.size) == 3, 'invalid terrain selection')
        for start, count in zip(self.origin, self.size):
            integer(start, 0, 32767)
            integer(count, 1, 128)
            require(start + count <= 32768, 'terrain selection overflow')
        require(self.volume <= routes.MAX_TILES, 'terrain selection exceeds profile')

    @property
    def volume(self) -> int:
        return self.size[0] * self.size[1] * self.size[2]

    @property
    def capture_limit(self) -> int:
        return max(1024, 575 + 20 * self.volume)

    def json(self) -> dict:
        return {'origin': list(self.origin), 'size': list(self.size)}


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    operations_token: bytes = field(repr=False)
    map_token: bytes = field(repr=False)

    @classmethod
    def load(cls) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'route inventory requires an isolated read-only environment')
        require(os.environ.get(OPT_IN) == '1', 'route inventory development opt-in required')
        tokens = tuple(os.environ.get(key, '').encode('utf-8')
                       for key in (wire.OPERATIONS_TOKEN, MAP_TOKEN))
        require(all(32 <= len(token) <= 256 and b'\0' not in token for token in tokens),
                'invalid route inventory read credential')
        return cls(wire.endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), *tokens)

    def guard(self) -> None:
        require(Authority.load() == self, 'route inventory authority changed')


class Client(wire.Client):
    """Reuse strict operations framing/paging/release without receipt authority.

    The receipt client's constructor is NOT called. Only these four fixed read
    methods are bound. Map replies alone get their region-derived byte ceiling;
    existing operations calls keep their original page limits and validators.
    """
    def __init__(self, authority: Authority, region: Region, budget: wire.Budget):
        # Trusted embedding owners may narrow authority through a subtype. The
        # CLI still loads its exact read-only environment; native methods and
        # credential fields remain the same four fixed read bindings below.
        require(isinstance(authority, Authority) and type(region) is Region,
                'closed route inventory authority and region required')
        region.__post_init__()
        authority.guard()
        budget.remaining()
        require(budget.calls >= MINIMUM_CALLS and budget.work_steps > 0
                and budget.network_bytes >= 24 + 2 * region.capture_limit + wire.MAX_REPLY,
                'insufficient allowance for the complete read bracket')
        self.authority, self.region, self.budget = authority, region, budget
        self.nonce, self.methods, self.manifests = secrets.token_bytes(32), {}, {}
        self.closed, self.used, self.map_reads = False, False, 0
        self.notifications_left = 2 * 1024 * 1024
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.socket.settimeout(budget.remaining())
            self.socket.connect(authority.address)
            self._send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self._read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'wrong native greeting')
            for family, name in BINDINGS:
                plugin, package, _ = PROFILES[family]
                bound = wire.decode(self._frame(0, wire.encode({1: name.encode('ascii'),
                    2: (package + '.Request').encode('ascii'), 3: (package + '.Reply').encode('ascii'),
                    4: plugin.encode('ascii')})), 1)
                require(set(bound) == {1}, 'invalid native method binding')
                identity = integer(bound[1], 2, 32767)
                require(identity not in self.methods.values(), 'aliased native methods')
                self.methods[family, name] = identity
            self._call('operations', 'Handshake', {}, set(range(1, 9)))
            self._map_call(False)
        except BaseException:
            self.close()
            raise

    def _call(self, family: str, name: str, extra: dict, expected_fields: set) -> dict:
        try:
            require(family == 'operations' and (family, name) in BINDINGS,
                    'method outside route inventory read profile')
            return super()._call(family, name, extra, expected_fields)
        except BaseException:
            self.close()
            raise

    def _map_frame(self, name: str, payload: bytes) -> bytes:
        require(not self.closed and name in ('Handshake', 'ReadObservation') and len(payload) <= 2048,
                'closed or invalid map read')
        self.authority.guard()
        self.budget.reserve('calls', 1)
        self._send(struct.pack('<h2xi', self.methods['map', name], len(payload)) + payload)
        notified, count = 0, 0
        maximum = self.region.capture_limit + 1024
        while True:
            self.authority.guard()
            kind, size = struct.unpack('<h2xi', self._read(8))
            require(kind in (-1, -3) and 0 <= size <= (maximum if kind == -1 else 65536),
                    'invalid map reply frame')
            if kind == -3:
                notified += size
                count += 1
                require(count <= 8 and notified <= 262144 and size <= self.notifications_left,
                        'map notification allowance exhausted')
                self.notifications_left -= size
            raw = self._read(size)
            if kind == -1:
                return raw

    def _map_call(self, observe: bool) -> bytes | None:
        try:
            require(type(observe) is bool and (not observe or self.map_reads < 2),
                    'map read allowance exhausted')
            if observe:
                self.map_reads += 1
            fields = {1: self.authority.map_token, 2: self.nonce, 3: 1, 4: 5,
                      **dict(zip(range(5, 11), self.region.origin + self.region.size)),
                      11: self.region.capture_limit}
            raw = self._map_frame('ReadObservation' if observe else 'Handshake', wire.encode(fields))
            reply = wire.decode(raw, 9, maximum_bytes=self.region.capture_limit + 1024)
            require(set(reply) == set(range(1, 10 if observe else 9)), 'invalid map reply fields')
            for number in (1, 2, 4, 5, 6):
                require(type(reply[number]) is int, 'invalid map scalar')
            for number in (3, 7, 8):
                require(type(reply[number]) is bytes, 'invalid map text')
            require(reply[1] == 1 and reply[2] == 0 and reply[3] == self.nonce
                    and reply[4] == 1 and reply[5] == 5, 'map refused or changed nonce/profile')
            manifest = Manifest(reply[6], wire.text(reply[7], 128), wire.text(reply[8], 128))
            manifest.encode()
            require('map' not in self.manifests or manifest == self.manifests['map'],
                    'map incarnation changed')
            require(manifest.software == self.manifests['operations'].software,
                    'map and operations software disagree')
            capture = reply.get(9)
            if observe:
                require(type(capture) is bytes and 1 <= len(capture) <= self.region.capture_limit,
                        'map capture exceeds selection bound')
            self.manifests['map'] = manifest
            self.authority.guard()
            self.budget.remaining()
            return capture
        except BaseException:
            self.close()
            raise

    def capture_once(self) -> tuple[Manifest, bytes, Manifest, bytes]:
        require(not self.closed and not self.used, 'route inventory acquisition already consumed')
        self.used = True
        try:
            before = self._map_call(True)
            operations = self._capture_operations()  # Every page + digest + original-token release.
            after = self._map_call(True)
            require(before == after, 'map endpoints differ around the inventory')
            self.authority.guard()
            self.budget.remaining()
            result = self.manifests['map'], before, self.manifests['operations'], operations
            self.close()  # No native connection remains during CPU analysis/publication.
            return result
        except BaseException:
            self.close()
            raise


def candidates_from(observed, request: Request, work) -> tuple[tuple[Candidate, ...], dict]:
    """Same conservative direct-ground singleton policy as furniture_inventory.

    Consume only a completely decoded operations roster, including relations in
    its final page. Every item gets one primary rejection/classification reason.
    """
    linked, containing = set(), set()
    for _, identity, _, _ in observed.attachments:
        work()
        linked.add(identity)
    for value in observed.items.values():
        work()
        if value.container is not None:
            containing.add(value.container)
    kinds = {'BED': 'bed', 'CHAIR': 'chair', 'TABLE': 'table'}
    counts = dict.fromkeys(('unsupported_kind', 'projected_flags', 'related_item', 'non_singleton',
                           'unknown_material', 'invalid_position', 'excluded', 'candidate'), 0)
    candidates, excluded = [], set(request.excluded_items)
    for value in observed.items.values():
        work()
        if value.kind not in kinds:
            reason = 'unsupported_kind'
        elif value.flags != 64:  # Only on_ground; all other projected flags exclude.
            reason = 'projected_flags'
        elif (value.container is not None or value.holder is not None
              or value.id in linked or value.id in containing):
            reason = 'related_item'
        elif value.stack != 1:
            reason = 'non_singleton'
        elif value.material < 0:
            reason = 'unknown_material'
        elif not all(0 <= coordinate <= 32767 for coordinate in value.position):
            reason = 'invalid_position'
        elif value.id in excluded:
            reason = 'excluded'
        else:
            reason = 'candidate'
            candidates.append(Candidate(value.id, kinds[value.kind], value.position,
                                        (value.material, value.material_index), value.subtype))
        counts[reason] += 1
    return tuple(candidates), counts


def packet(result: dict | None, request: Request | None = None, source: dict | None = None) -> dict:
    success = result is not None
    sequence = result.get('delivery_sequence') if success else None
    optimization = result.get('delivery_optimization') if success else None
    return {'schema': 'dfmcp.route-furniture-inventory-result/1', 'ok': success,
            'result': result, 'request': request.json() if success else None,
            'source': source if success else None,
            'native_capture_established': success, 'capture_release_verified': success,
            'atomic_cross_profile_snapshot_proven': False, 'runtime_admitted': False,
            **({} if success else {'error': {'code': 'ROUTE_FURNITURE_REFUSED',
                'detail': 'Request, source, authority, evidence or budget refused. No allocation established; no placement was attempted.'}}),
            'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'furniture.allocate_routes',
                'phase': 'propose', 'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
                'continuity': {'status': 'bootstrap' if success else 'indeterminate', 'basis': None, 'gap': None},
                'profile': 'tactical', 'briefing': {'status': result['status'] if success else 'unestablished',
                    'runtime_admitted': False, 'mutation_admissible': False, 'items_reserved': False,
                    **({'delivery_sequence': sequence['status']} if sequence is not None else {}),
                    **({'delivery_cost_optimality_proven': optimization['cost_optimality_proven'],
                        'delivery_cost_gap': optimization['absolute_cost_gap']}
                       if optimization is not None else {})},
                'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
                'uncertainty': ['Equal paused map endpoints bracket a released inventory, not an atomic cross-profile snapshot or continuous stability.',
                    'Routes use a bounded dry floor/stair model, not native unit pathfinding, danger, or outside-region reachability.',
                    ('Delivery sequencing checks only a captured candidate model, not future routes. Fresh original-selection review remains required.'
                     if sequence is not None else
                     'The exact-item plan does not retain future route constraints. Earlier placements may obstruct later routes; fresh native review remains required.'),
                    'No placement receipts or active effects were queried. This proposal cannot clear uncertainty, reserve items, or authorize placement.',
                    *(['Delivery-cost optimality covers only the captured target-removal model. It is not native hauling time; an incomplete search may retain a complete but unproven proposal.']
                      if optimization is not None else [])],
                'coverage': {'scope': 'bracketed_bounded_map_and_complete_operations_inventory',
                    'native_acquisition': 'established_by_this_call' if success else 'unestablished',
                    'active_effects': 'not_queried', 'continuous_history': False,
                    'native_pathfinding_proven': False, 'candidate_policy': CANDIDATE_POLICY},
                'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False},
                'references': [source] if success else []}}


def bounded_output(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete route inventory response exceeds output bound')
    return raw


def run(request: Request, region: Region, authority: Authority, budget: wire.Budget,
        *, max_route_steps: int = routes.MAX_ROUTE_STEPS, sequence_deliveries: bool = False,
        minimize_delivery_cost: bool = False, require_optimal_delivery: bool = False,
        max_search_nodes: int = optimal.MAX_SEARCH_NODES) -> bytes:
    require(type(request) is Request and type(region) is Region and type(authority) is Authority,
            'closed route inventory inputs required')
    require(type(sequence_deliveries) is bool, 'delivery sequencing requires an explicit boolean')
    require(type(minimize_delivery_cost) is bool and (not minimize_delivery_cost or sequence_deliveries),
            'delivery cost optimization requires explicit sequencing opt-in')
    require(type(require_optimal_delivery) is bool and (not require_optimal_delivery or minimize_delivery_cost),
            'strict delivery optimality requires cost optimization')
    integer(max_search_nodes, 1, optimal.MAX_SEARCH_NODES)
    require(minimize_delivery_cost or max_search_nodes == optimal.MAX_SEARCH_NODES,
            'delivery search limits require cost optimization')
    request.__post_init__()
    region.__post_init__()
    integer(max_route_steps, 0, routes.MAX_ROUTE_STEPS)
    authority.guard()
    budget.remaining()
    with Client(authority, region, budget) as client:
        map_manifest, map_raw, operations_manifest, operations_raw = client.capture_once()
    require(client.closed, 'native source must close before projection')
    terrain = routes.decode_terrain(map_raw, budget.work)
    require((terrain.origin, terrain.size) == (region.origin, region.size), 'native map substituted selection')
    observed = decode_operations(operations_raw, budget.work)
    require((terrain.folder, terrain.site) == (observed.folder, observed.site) == (request.folder, request.site),
            'source fortress mismatch')
    require(terrain.paused and observed.paused and terrain.tick == observed.tick,
            'map and inventory must share one paused tick')
    require(map_manifest.software == operations_manifest.software, 'source software mismatch')
    selected, counts = candidates_from(observed, request, budget.work)
    # Reserve the solver's complete remaining allowance up front. Its internal
    # counter charges EVERY unit; the outer callback only checks wall/authority.
    # No work allowance is replenished between projection and optimization.
    solver_allowance = min(routes.MAX_WORK, budget.work_steps)
    require(solver_allowance > 0, 'no route solver allowance remains')
    budget.work_steps -= solver_allowance

    def guard() -> None:
        budget.remaining()
        authority.guard()

    if minimize_delivery_cost:
        request, result = optimal.allocate_optimal_routes(
            request, selected, map_raw, guard, max_route_steps=max_route_steps,
            maximum_work=solver_allowance, max_search_nodes=max_search_nodes,
            require_optimal=require_optimal_delivery)
    elif sequence_deliveries:
        request, result = allocate_sequenced_routes(
            request, selected, map_raw, guard,
            max_route_steps=max_route_steps, maximum_work=solver_allowance)
    else:
        result = routes.allocate_routes(request, selected, map_raw, guard,
                                        max_route_steps=max_route_steps, maximum_work=solver_allowance)
    source = {'policy': ACQUISITION_POLICY, 'endpoint': f'{authority.address[0]}:{authority.address[1]}',
              'game_tick': observed.tick, 'paused_at_capture': True,
              'map': {'profile': 'map/1.5', 'sha256': terrain.digest, 'bytes': len(map_raw),
                      'generation': map_manifest.generation, 'region': region.json()},
              'inventory': {'profile': 'operations/1.4', 'sha256': observed.digest, 'bytes': len(operations_raw),
                            'generation': operations_manifest.generation, 'horizons': list(observed.horizons)},
              'df_version': map_manifest.df_version, 'dfhack_version': map_manifest.dfhack_version,
              'projection': {'policy': CANDIDATE_POLICY, 'counts': counts,
                             'complete_roster_decoded': True, 'jobs': len(observed.jobs),
                             'buildings': len(observed.buildings), 'items': len(observed.items),
                             'attachments': len(observed.attachments)},
              'pure_allocation_digest': result['decision_digest'],
              'solver_work_reserved': solver_allowance,
              'atomic_cross_profile_snapshot_proven': False,
              'native_acquisition_independently_attested': False}
    source['evidence_digest'] = hashlib.sha256(b'dfmcp-route-inventory-evidence/1\0' + canonical(source)).hexdigest()
    output = bounded_output(packet(result, request, source))
    guard()  # Cached results cannot outlive the configuration used to acquire them.
    return output
