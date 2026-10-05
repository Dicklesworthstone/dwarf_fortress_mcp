"""Read-only operations/1.4 inventory adapter for whole-plan furniture proposals.

Reuse the existing full-roster decoder and exact retained-page/release transport.
Bind ONLY operations Handshake and ReadObservation. No furniture credential,
placement method, native reservation, monitor receipt or world-anchor invention.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import os
import secrets
import socket
import struct

from construction_monitor_rpc import (
    Budget, Client as ReceiptClient, OPERATIONS_TOKEN, PROFILES, decode, encode, endpoint,
)
from construction_receipt import Manifest, decode_operations
from furniture_allocation import Candidate, Guard, Request, allocate
from furniture_plan import canonical, integer, require
from furniture_handoff import Handoff, InventorySource, Selected

OPT_IN = 'DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION'
ENDPOINT = 'DFMCP_FURNITURE_ALLOCATION_ENDPOINT'
ENVIRONMENT = (OPT_IN, ENDPOINT, OPERATIONS_TOKEN)
BINDINGS = (('operations', 'Handshake'), ('operations', 'ReadObservation'))
POLICY = 'dfmcp.furniture-inventory-candidates/1'
MAX_OUTPUT = 65536


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    operations_token: bytes = field(repr=False)

    @classmethod
    def load(cls) -> Authority:
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'allocation accepts only its isolated read environment')
        require(os.environ.get(OPT_IN) == '1', 'explicit allocation development opt-in required')
        token = os.environ.get(OPERATIONS_TOKEN, '').encode('utf-8')
        require(32 <= len(token) <= 256 and b'\0' not in token, 'invalid operations query credential')
        return cls(endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), token)

    def guard(self) -> None:
        require(Authority.load() == self, 'operator inventory-read configuration changed')


class InventoryClient(ReceiptClient):
    """Narrow constructor over unchanged framing, paging and verified release.

    The parent receipt constructor is deliberately NOT called: inventory planning
    has neither a placement goal nor a build credential. The parent page reader
    is used unchanged; it cannot return before all pages, digest and release pass.
    """
    def __init__(self, authority: Authority, budget: Budget):
        authority.guard()
        budget.remaining()
        self.authority, self.budget = authority, budget
        self.nonce, self.methods = secrets.token_bytes(32), {}
        self.closed, self.used, self.notifications_left = False, False, 2 * 1024 * 1024
        self.manifests = {}
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.socket.settimeout(budget.remaining())
            self.socket.connect(authority.address)
            self._send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self._read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'wrong native greeting')
            for family, name in BINDINGS:
                plugin, package, _ = PROFILES[family]
                reply = decode(self._frame(0, encode({1: name.encode('ascii'),
                    2: (package + '.Request').encode('ascii'), 3: (package + '.Reply').encode('ascii'),
                    4: plugin.encode('ascii')})), 1)
                require(set(reply) == {1}, 'invalid native inventory method binding')
                identity = integer(reply[1], 2, 32767)
                require(identity not in self.methods.values(), 'aliased native inventory methods')
                self.methods[family, name] = identity
            self._call('operations', 'Handshake', {}, set(range(1, 9)))
        except BaseException:
            self.close()
            raise

    def _call(self, family: str, name: str, extra: dict, expected_fields: set) -> dict:
        try:
            require((family, name) in BINDINGS, 'method outside inventory-read profile')
            return super()._call(family, name, extra, expected_fields)
        except BaseException:
            self.close()
            raise

    def capture_once(self) -> tuple[Manifest, bytes]:
        require(not self.used and not self.closed, 'inventory acquisition already consumed or closed')
        self.used = True
        try:
            raw = self._capture_operations()
            self.authority.guard()
            self.budget.remaining()
            return self.manifests['operations'], raw
        except BaseException:
            self.close()
            raise


def project(request: Request, raw: bytes, guard: Guard,
            *, handoff_binding: tuple[str, Manifest] | None = None) -> dict:
    """Decode all bytes before projection. Supplied bytes alone do not prove I/O."""
    require(type(request) is Request, 'invalid inventory request')
    request.__post_init__()
    if handoff_binding is not None:
        require(type(handoff_binding) is tuple and len(handoff_binding) == 2
                and type(handoff_binding[0]) is str and type(handoff_binding[1]) is Manifest,
                'invalid handoff source binding')
        endpoint(handoff_binding[0])
        handoff_binding[1].encode()
    observed = decode_operations(raw, guard)
    require((observed.folder, observed.site) == (request.folder, request.site),
            'inventory is not from the requested fortress')
    linked, containing = set(), set()
    for _, identity, _, _ in observed.attachments:
        guard()
        linked.add(identity)
    for value in observed.items.values():
        guard()
        if value.container is not None:
            containing.add(value.container)
    kinds = {'BED': 'bed', 'CHAIR': 'chair', 'TABLE': 'table'}
    counts = dict.fromkeys(('unsupported_kind', 'projected_flags', 'related_item',
                           'non_singleton', 'unknown_material', 'invalid_position', 'excluded', 'candidate'), 0)
    candidates, excluded = [], set(request.excluded_items)
    for value in observed.items.values():
        guard()
        if value.kind not in kinds:
            reason = 'unsupported_kind'
        # Native field order: forbid, in_job, dump, removed, rotten, trader,
        # on_ground, in_inventory, in_building. Only on_ground may be set.
        elif value.flags != (1 << 6):
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
    result = allocate(request, tuple(candidates), guard)
    result['request'] = request.json()
    result['source'] = {'schema': 'dfmcp.operations-capture-reference/1', 'profile': 'operations/1.4',
                        'capture_sha256': observed.digest, 'capture_bytes': len(raw),
                        'game_tick': observed.tick, 'paused_at_capture': observed.paused,
                        'world_folder': observed.folder, 'site': observed.site,
                        'native_horizons': list(observed.horizons)}
    result['projection'] = {'policy': POLICY, 'complete_roster_decoded': True,
                            'jobs': len(observed.jobs), 'buildings': len(observed.buildings),
                            'items': len(observed.items), 'attachments': len(observed.attachments),
                            'counts': counts, 'unprojected_item_state': 'unknown',
                            'terrain_and_map_dimensions': 'not_observed', 'placement_state': 'not_queried'}
    if handoff_binding is not None:
        address, manifest = handoff_binding
        result['schema'] = 'dfmcp.furniture-allocation-handoff-result/1'
        result['handoff'] = None
        if result['status'] == 'allocated':
            source = InventorySource(address, manifest.generation, manifest.df_version,
                manifest.dfhack_version, observed.digest, len(raw), observed.tick, observed.horizons)
            selected = []
            for assignment in result['assignments']:
                guard()
                item = observed.items[assignment['item']]
                selected.append(Selected(assignment['slot'],
                    Candidate(item.id, kinds[item.kind], item.position,
                              (item.material, item.material_index), item.subtype), item.native_type))
            handoff = Handoff(request, source, tuple(selected))
            require(handoff.plan().json() == result['plan'], 'handoff differs from complete allocation')
            result['handoff'], result['handoff_digest'] = handoff.json(), handoff.digest
            # Full request and original selections live in the handoff; do not duplicate
            # them and consume the output budget needed for the executable plan.
            del result['request']
            del result['assignments']
            result['allocation_details_location'] = 'handoff'
    guard()
    return result


def packet(result: dict | None, error_class: str | None = None) -> dict:
    success = result is not None
    status = result['status'] if success else 'unestablished'
    retained = success and 'handoff' in result
    return {'ok': success, 'profile': 'furniture-allocation-handoff/1' if retained else 'furniture-allocation/1',
            'runtime_admitted': False,
            'result': result, **({} if success else {'error_class': error_class,
                'detail': 'Inventory, request, authority, budget or evidence refused; no allocation established.'}),
            'agent_turn': {
                'schema': 'dfmcp.agent_turn/1', 'operation': 'furniture.allocate', 'phase': 'propose',
                'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
                'continuity': {'status': 'bootstrap' if success else 'indeterminate',
                               'basis': None, 'gap': None, 'reset_reason': None},
                'profile': 'tactical',
                'briefing': {'status': status, 'runtime_admitted': False, 'mutation_admissible': False,
                             'items_reserved': False, 'construction_completion_proven': False},
                'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
                'uncertainty': ['Allocation is a historical proposal, not a reservation or placement permission.',
                                'Wear, unprojected flags, terrain, dimensions and worker paths are not established.',
                                ('Retained constraints narrow future review; the exported artifact does not independently attest acquisition.'
                                 if retained else 'Material and subtype constraints are assessed at this capture only; review each later native plan.'),
                                'Existing placement uncertainty is not queried or cleared.'],
                'coverage': {'operations_capture': 'complete' if success else 'unestablished',
                             'active_work': 'not_queried', 'placement_receipts': 'not_queried',
                             'candidate_policy': POLICY},
                'budget': {'output_bytes_limit': MAX_OUTPUT, 'token_count_measured': False},
                'references': [result['source']] if success else []}}


def bounded_output(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete inventory allocation output exceeds 64 KiB')
    return raw


def run(request: Request, authority: Authority, budget: Budget, *, retain_constraints: bool = False) -> bytes:
    """One live foreground read and complete serialization before publication."""
    require(type(request) is Request, 'invalid inventory request')
    request.__post_init__()
    require(type(retain_constraints) is bool, 'invalid handoff export option')
    with InventoryClient(authority, budget) as client:
        manifest, raw = client.capture_once()
        if retain_constraints:
            address = f'{authority.address[0]}:{authority.address[1]}'
            result = project(request, raw, budget.work, handoff_binding=(address, manifest))
        else:
            result = project(request, raw, budget.work)
        result['source'].update(native_generation=manifest.generation,
                                df_version=manifest.df_version, dfhack_version=manifest.dfhack_version)
        result['native_capture_established'] = True
        result['capture_release_verified'] = True
        output = bounded_output(packet(result))
        # Cached allocation facts must not bypass a revoked read configuration.
        authority.guard()
        budget.remaining()
        return output
