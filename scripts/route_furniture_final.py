"""Fresh construction observations after ALL original placements were registered.

One query-only connection brackets a complete operations roster and equal map
endpoints with every original native receipt. No next placement, synthetic goal,
new native method, clock effect, or completion/stability permit is invented.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import struct

from build_placement_wire import KINDS, MAX_RECORD_BYTES, Record
import construction_monitor_rpc as wire
import construction_prefix as construction
from construction_receipt import Manifest, decode_operations
from furniture_allocation import Candidate
from furniture_plan import canonical, integer, require
import route_furniture_allocation as routes
from route_furniture_review import Review, Source, candidate, digest_text

POLICY = 'all-original-placement-receipts-around-paused-final-construction/1'
SCHEMA = 'dfmcp.route-furniture-final-frontier/1'
MAX_OUTPUT = 96 * 1024
ANALYSIS_WORK = 6_000_000
FINAL_WORK = 100_000
MAP_METHODS = ('Handshake', 'ReadObservation')
STATUS = {'pending': 'construction_pending', 'blocked': 'construction_blocked',
          'unestablished': 'construction_unestablished', 'quiescent': 'construction_quiescent'}


def original_selection(batch):
    """Authenticate the entire original selection through the held batch owner.

    This consumes the actual owner's audited inventory, not caller-selected IDs
    or a subset of available receipts. It is reusable by final-clock scope guards.
    """
    budget = batch.budget
    budget.work()
    require(not batch.writable and type(batch.review) is Review,
            'final construction requires read-only original batch custody')
    original, request, source, plan = batch.review.unpack(budget.work)
    require(plan == batch.plan and request == batch.request and source == batch.source,
            'final construction differs from the immutable original review')
    digest_text(batch.id)
    state = batch.audit()
    require(state['inventory_verified'] is True and state['batch_id'] == batch.id
            and state['review_digest'] == batch.review.digest
            and state['status'] == 'all_placed' and state['pending_step'] is None
            and state['next_step'] is None and state['stopped'] is False
            and state['placed'] == len(plan.steps) == len(batch.entries)
            and state['plan_digest'] == plan.digest and state['plan'] == plan.json(),
            'final construction requires the complete unstopped original placement inventory')
    digest_text(state['inventory_digest'])
    require(type(state['steps']) is list and len(state['steps']) == len(plan.steps)
            and {row['name'] for row in state['steps']} == {step.name for step in plan.steps}
            and all(row['phase'] == 'placed' and row.get('registered') is True for row in state['steps']),
            'final construction cannot omit or adopt unregistered original work')
    receipts = batch.placed_prefix()
    require(type(receipts) is tuple and len(receipts) == len(plan.steps),
            'final construction needs every original receipt in order')
    saved = {row['slot']: row for row in original['selections']}
    slots = {slot.name: slot for slot in request.slots}
    build = Manifest(batch.native.generation, batch.native.df_version, batch.native.dfhack_version)
    build.encode()
    require(build.software == (source.df_version, source.dfhack_version)
            and wire.endpoint(source.endpoint) == batch.address, 'final construction source differs')
    previous = None
    for step, raw in zip(plan.ordered, receipts):
        budget.work()
        require(type(raw) is bytes and 0 < len(raw) <= MAX_RECORD_BYTES, 'invalid original receipt extent')
        record = Record.decode(raw)
        before = record.plan.before
        row, item = saved[step.name], before.item
        selected = candidate(row['candidate'])
        current = Candidate(step.item, KINDS[item.kind], item.pos,
                            (item.material, item.material_index), item.subtype)
        require(record.phase == 'placed' and record.plan.key == 'frt-' + batch.id + '-' + step.name
                and (before.selection.kind, before.selection.item, before.selection.target)
                == (KINDS.index(step.kind), step.item, step.target)
                and (current.id, current.kind, current.material, current.subtype, item.native_type)
                == (selected.id, selected.kind, selected.material, selected.subtype, row['native_type'])
                and routes.distance(slots[step.name], current) is not None,
                'final receipt changed an original item, target, key or constraint')
        require((before.generation, before.folder, before.site, before.dimensions)
                == (build.generation, request.folder, request.site, source.dimensions)
                and before.tick >= source.tick and before.next_job >= source.horizons[0]
                and before.next_building >= source.horizons[1], 'original receipt source regressed')
        if previous is not None:
            require(before.tick >= previous.tick and before.sequence >= previous.sequence
                    and before.next_job >= previous.next_job and before.next_building >= previous.next_building,
                    'original final-placement chronology regressed')
        previous = record.after
    batch.check()
    return state, receipts, build


@dataclass(frozen=True)
class _ReceiptSelection:
    """The existing read client's stateless receipt port, NOT a temporal Goal."""
    receipt: bytes

    @property
    def record(self):
        value = Record.decode(self.receipt)
        require(value.phase == 'placed', 'final construction cannot query uncertain original work')
        return value

    def encode(self):
        return self.record.raw


class _ReadAuthority:
    def __init__(self, authority):
        self.original = authority
        self.address = authority.address
        self.build_token = authority.token
        self.operations_token = authority.operations_token
        self.map_token = authority.map_token

    def guard(self):
        self.original.guard('QueryPlacement')


class Client(wire.Client):
    """Reuse receipt framing/paging/release; add only the unchanged map/1.5 reads."""
    def __init__(self, authority, receipts: tuple[bytes, ...], source: Source, build: Manifest, budget):
        require(type(source) is Source and type(build) is Manifest
                and type(receipts) is tuple and 1 <= len(receipts) <= 32,
                'closed complete final-construction source required')
        source.__post_init__()
        build.encode()
        require(wire.endpoint(source.endpoint) == authority.address
                and build.software == (source.df_version, source.dfhack_version),
                'final construction endpoint or software differs')
        self.members = tuple(_ReceiptSelection(raw) for raw in receipts)
        for member in self.members:
            budget.work()
            require(type(member.receipt) is bytes and 0 < len(member.receipt) <= MAX_RECORD_BYTES,
                    'final receipt exceeds native bound')
            require(member.record.plan.before.generation == build.generation,
                    'final receipt belongs to another native incarnation')
        self.expected_source, self.expected_build = source, build
        self.map_reads = 0
        self.map_limit = max(1024, 575 + 20 * source.size[0] * source.size[1] * source.size[2])
        # Six binds, three handshakes, two maps, one page, release, all receipts
        # twice. Further pages consume the SAME nonrenewable budget as usual.
        require(budget.calls >= 13 + 2 * len(receipts)
                and budget.network_bytes >= 2 * sum(map(len, receipts)) + 2 * self.map_limit + wire.MAX_REPLY + 8192,
                'complete final-construction bracket cannot fit')
        super().__init__(_ReadAuthority(authority), self.members[0], budget)
        try:
            require(self.manifests['build'] == build
                    and self.manifests['operations'] == Manifest(source.inventory_generation, *build.software),
                    'final construction original native sources changed')
            for name in MAP_METHODS:
                bound = wire.decode(self._frame(0, wire.encode({1: name.encode('ascii'),
                    2: b'dfmcp.map.v1_5.Request', 3: b'dfmcp.map.v1_5.Reply',
                    4: b'dfmcp_map_v1_5'})), 1)
                require(set(bound) == {1}, 'invalid final map binding')
                method = integer(bound[1], 2, 32767)
                require(method not in self.methods.values(), 'aliased final read binding')
                self.methods['map', name] = method
            self._map(False)
        except BaseException:
            self.close()
            raise

    def _call(self, family, name, extra, expected_fields):
        require((family, name) in wire.BINDINGS, 'final construction is query-only')
        reply = super()._call(family, name, extra, expected_fields)
        require(family != 'build' or reply[12] == 0, 'native unresolved work blocks final construction')
        return reply

    def _map_frame(self, name, payload):
        require(not self.closed and name in MAP_METHODS and len(payload) <= 2048,
                'closed or invalid final map read')
        self.authority.guard()
        self.budget.reserve('calls', 1)
        self._send(struct.pack('<h2xi', self.methods['map', name], len(payload)) + payload)
        count, notified = 0, 0
        while True:
            self.authority.guard()
            kind, size = struct.unpack('<h2xi', self._read(8))
            require(kind in (-1, -3) and 0 <= size <= (self.map_limit + 1024 if kind == -1 else 65536),
                    'oversized or invalid final map frame')
            if kind == -3:
                count += 1
                notified += size
                require(count <= 8 and notified <= 262144 and size <= self.notifications_left,
                        'final map notifications exhausted')
                self.notifications_left -= size
            raw = self._read(size)
            if kind == -1:
                return raw

    def _map(self, observe):
        require(type(observe) is bool and (not observe or self.map_reads < 2), 'final map read already consumed')
        self.map_reads += int(observe)
        source = self.expected_source
        request = {1: self.authority.map_token, 2: self.nonce, 3: 1, 4: 5,
                   **dict(zip(range(5, 11), source.origin + source.size)), 11: self.map_limit}
        raw = self._map_frame('ReadObservation' if observe else 'Handshake', wire.encode(request))
        reply = wire.decode(raw, 9, maximum_bytes=self.map_limit + 1024)
        require(set(reply) == set(range(1, 10 if observe else 9)), 'invalid final map reply shape')
        require(all(type(reply[n]) is int for n in (1, 2, 4, 5, 6))
                and all(type(reply[n]) is bytes for n in (3, 7, 8)), 'invalid final map field types')
        require((reply[1], reply[2], reply[3], reply[4], reply[5]) == (1, 0, self.nonce, 1, 5),
                'final map refusal or wrong nonce/profile')
        manifest = Manifest(reply[6], wire.text(reply[7], 128), wire.text(reply[8], 128))
        require(manifest == Manifest(source.map_generation, source.df_version, source.dfhack_version),
                'final map source changed')
        capture = reply.get(9)
        require(not observe or (type(capture) is bytes and 0 < len(capture) <= self.map_limit),
                'final map capture exceeds bound')
        self.authority.guard()
        self.budget.remaining()
        return capture

    def _receipts(self):
        for member in self.members:
            self.budget.work()
            record = member.record
            reply = self._call('build', 'QueryPlacement',
                {10: record.plan.key.encode('ascii'), 12: record.plan.digest},
                set(range(1, 9)) | {10, 12, 13})
            require(reply[13] > 0 and reply[10] == member.receipt,
                    'complete original final receipt not retained byte-for-byte')

    def capture_once(self):
        require(not self.used and not self.closed, 'final construction acquisition already consumed')
        self.used = True
        try:
            self._receipts()
            before = self._map(True)
            raw = self._capture_operations()
            after = self._map(True)
            self._receipts()
            require(before == after, 'final construction map endpoints changed')
            self.authority.guard()
            self.budget.remaining()
            self.close()
            return before, raw
        except BaseException:
            self.close()
            raise


def read_frontier(batch, authority):
    """Inspect ALL original furnishings, even though there is no next placement.

    This is a sampled condition, not the stability-qualified original goal. The
    existing route-furniture completion monitor retains its own timing/evidence.
    """
    authority.guard('QueryPlacement')
    state, receipts, build = original_selection(batch)
    budget = batch.budget
    require(budget.work_steps > ANALYSIS_WORK + FINAL_WORK, 'final construction analysis cannot fit')
    with Client(authority, receipts, batch.source, build, budget) as client:
        map_raw, operations_raw = client.capture_once()
    require(client.closed, 'final construction source must close before analysis')
    terrain = routes.decode_terrain(map_raw, budget.work)
    observed = decode_operations(operations_raw, budget.work)
    require((terrain.folder, terrain.site) == (observed.folder, observed.site)
            == (batch.request.folder, batch.request.site)
            and terrain.paused and observed.paused and terrain.tick == observed.tick,
            'final construction sources do not share the original paused fortress')
    source = Source(batch.source.endpoint, *build.software, batch.source.map_generation,
                    batch.source.inventory_generation, terrain.digest, observed.digest,
                    observed.tick, terrain.dimensions, terrain.origin, terrain.size, observed.horizons)
    batch.source.permits(source)
    # Reserve one fixed slice, never refund it. The predicate charges every unit
    # internally; its sampled callback only checks the original wall/authority.
    require(budget.work_steps > ANALYSIS_WORK + FINAL_WORK, 'final construction projection exhausted analysis')
    budget.work_steps -= ANALYSIS_WORK
    def guard():
        budget.remaining()
        authority.guard('QueryPlacement')
    report = construction.assess_prefix(receipts, operations_raw, guard, maximum_work=ANALYSIS_WORK)
    report.pop('work_units')
    report.pop('maximum_work')
    current, current_receipts, current_build = original_selection(batch)
    require(current['inventory_digest'] == state['inventory_digest'] and current_receipts == receipts
            and current_build == build, 'original construction inventory changed during acquisition')
    out = {'schema': SCHEMA, 'policy': POLICY, 'batch_id': batch.id,
           'review_digest': batch.review.digest, 'original_plan_digest': batch.plan.digest,
           'inventory_digest': state['inventory_digest'], 'next_step': None,
           'phase': 'final_construction', 'status': STATUS[report['status']],
           'source': source.json(), 'construction_prefix': report,
           'placement_manifest': {'generation': build.generation, 'df_version': build.df_version,
                                  'dfhack_version': build.dfhack_version},
           'all_original_receipts_checked': True, 'native_receipt_bracket_verified': True,
           'all_original_furnishings_quiescent_now': report['status'] == 'quiescent',
           'atomic_cross_profile_snapshot_proven': False, 'construction_completion_proven': False,
           'continuous_stability_proven': False, 'placement_authorized': False,
           'retry_permitted': False, 'production_admitted': False}
    out['evidence_digest'] = hashlib.sha256(b'dfmcp-final-construction-frontier/1\0' + canonical(out)).hexdigest()
    require(len(canonical(out)) <= MAX_OUTPUT, 'complete final construction output exceeds bound')
    state.update(route_frontier=out, advance_allowed=False, expected_plan=None, confirm_review=None,
                 execution_review_required=True, route_constraints_checked_this_call=False)
    authority.guard('QueryPlacement')
    batch.check()
    budget.remaining()
    return state, out
