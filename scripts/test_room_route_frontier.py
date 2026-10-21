"""Actual room/map/review codecs; the prior execution report is an injected port.

These tests qualify the additional room gate, not native acquisition or the
route/construction producer. Integration tests exercise those owning call sites.
"""
from __future__ import annotations

from copy import deepcopy
from dataclasses import replace
import hashlib
import struct
import unittest

from furniture_allocation import Candidate
from furniture_plan import canonical
from room_provisioning import RoomPlan
from room_route_handoff import RoomRouteHandoff, _requirements
import room_route_frontier as gate
import route_furniture_allocation as routes
import route_furniture_review as reviews


DIMENSIONS = (64, 64, 8)
ORIGIN = (0, 0, 2)
SIZE = (32, 32, 1)


def map_bytes(room, changes=None, *, tick=10, folder='fort', site=7, paused=True,
              origin=ORIGIN, size=SIZE):
    changes = {} if changes is None else changes
    required = _requirements(room.json(), lambda: None)
    body = bytearray(b'DFMM1500' + struct.pack('>IIBI', 0, tick, paused, site))
    encoded = folder.encode()
    body += struct.pack('>H', len(encoded)) + encoded
    body += struct.pack('>10I', *DIMENSIONS, *origin, *size, size[0] * size[1] * size[2])
    for z in range(origin[2], origin[2] + size[2]):
        for y in range(origin[1], origin[1] + size[1]):
            for x in range(origin[0], origin[0] + size[0]):
                point = (x, y, z)
                # The test encoder is independent of the production map decoder.
                attrs = [1, required.get(point, 3), 0, 0, 0, 0, 0, 0, 1, 10015, 10015]
                change = changes.get(point, {})
                presence = change.get('presence', 2)
                body.append(presence)
                if presence == 2:
                    for index, value in change.items():
                        if index != 'presence':
                            attrs[index] = value
                    body += struct.pack('>IBBBBBBBIHH', *attrs)
    return bytes(body)


def fixture(*, rotation=0, count=1, dining=False):
    template = ({'kind': 'dining_hall', 'table_count': count, 'columns': 4} if dining else
                {'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': [3, 3]})
    room = RoomPlan.compile({'schema': 'dfmcp.room-provisioning-request/1', 'world_folder': 'fort',
        'site': 7, 'areas': [{'name': 'rooms', 'origin': [16, 16, 2] if rotation else [5, 5, 2],
                             'rotation': rotation, 'template': template}]})
    request = room.request()
    candidates = tuple(Candidate(100 + index, slot.kind, slot.target, (419, -1), -1)
                       for index, slot in enumerate(request.slots))
    types = {item.id: ('bed', 'chair', 'table').index(item.kind) for item in candidates}
    raw = map_bytes(room)
    source = reviews.Source('127.0.0.1:5000', 'df', 'dfhack', 3, 4,
        hashlib.sha256(raw).hexdigest(), 'a' * 64, 10, DIMENSIONS, ORIGIN, SIZE, (90, 70, 1000))
    allocation = routes.allocate_routes(request, candidates, raw)
    review = reviews.create(request, allocation, source, types)
    handoff = RoomRouteHandoff.create(room, review)
    return handoff, room, review, source, raw


def reseal(report):
    payload = {key: value for key, value in report.items()
               if key not in ('work_units', 'maximum_work', 'evidence_digest')}
    report['evidence_digest'] = hashlib.sha256(gate.DOMAIN + canonical(payload)).hexdigest()
    return report


def prior_report(review, source, *, status='ready', prefix=0):
    """Model the authenticated producer port; no receipt-authentication claim."""
    original, _, _, plan = review.unpack()
    return reseal({'schema': 'dfmcp.route-furniture-frontier/1', 'policy': gate.INPUT_POLICY,
        'status': status, 'batch_id': 'b' * 64, 'review_digest': review.digest,
        'original_plan_digest': plan.digest, 'original_plan': plan.json(),
        'next_step': plan.ordered[prefix].name,
        'placed_prefix': [{'slot': step.name, 'key': 'test-key-' + step.name,
                           'receipt_digest': 'c' * 64, 'native_plan_digest': 'd' * 64}
                          for step in plan.ordered[:prefix]],
        'source': source.json(), 'max_route_steps': original['max_route_steps'],
        'all_remaining_selections_checked': True, 'prefix_matches_route_frontier': True,
        'construction_prefix': {'status': 'no_predecessors' if prefix == 0 else 'quiescent',
                                'predecessor_gate_met': True},
        'placement_authorized': False, 'retry_permitted': False,
        'work_units': 123, 'maximum_work': 6_000_000})


class RoomFrontierTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.handoff, cls.room, cls.review, cls.source, cls.raw = fixture()

    def current(self, raw, **changes):
        source = replace(self.source, map_sha256=hashlib.sha256(raw).hexdigest(), **changes)
        return prior_report(self.review, source)

    def test_complete_room_extends_but_does_not_modify_execution_evidence(self):
        report = prior_report(self.review, self.source)
        saved = deepcopy(report)
        result = gate.gate_frontier(self.handoff, report, self.raw)
        self.assertEqual(report, saved)
        self.assertEqual(result['status'], 'ready')
        self.assertEqual(result['room_plan_digest'], self.room.digest)
        self.assertEqual(result['room_handoff_digest'], self.handoff.digest)
        self.assertEqual(result['execution_frontier_evidence_digest'], report['evidence_digest'])
        self.assertTrue(result['room_constraints_checked_this_call'])
        self.assertGreater(result['work_units'], report['work_units'])
        for field in ('placement_authorized', 'room_completion_proven', 'atomic_room_placement_proven',
                      'retry_permitted'):
            self.assertFalse(result[field])
        self.assertEqual(gate.gate_frontier(self.handoff, report, self.raw), result)

    def test_every_original_floor_and_wall_is_load_bearing(self):
        requirements = _requirements(self.room.json(), lambda: None)
        for point, shape in requirements.items():
            with self.subTest(point=point):
                raw = map_bytes(self.room, {point: {1: 2 if shape == 3 else 3}})
                result = gate.gate_frontier(self.handoff, self.current(raw), raw)
                self.assertEqual(result['status'], 'room_blocked')
                self.assertEqual(result['room_terrain']['deficits']['count'], 1)
                self.assertEqual(result['room_terrain']['evaluated_tiles'], len(requirements))

    def test_hidden_and_missing_are_unknown_not_matched_or_known_blockage(self):
        point = next(p for p, shape in _requirements(self.room.json(), lambda: None).items() if shape == 2)
        for presence in (0, 1):
            raw = map_bytes(self.room, {point: {'presence': presence}})
            result = gate.gate_frontier(self.handoff, self.current(raw), raw)
            self.assertEqual(result['status'], 'room_unestablished')
            self.assertFalse(result['room_terrain']['all_required_shapes_at_sample'])

    def test_liquid_designations_and_wall_occupancy_are_blocking(self):
        wall = next(p for p, shape in _requirements(self.room.json(), lambda: None).items() if shape == 2)
        for column in (2, 3, 5, 6, 7):
            raw = map_bytes(self.room, {wall: {column: 1}})
            result = gate.gate_frontier(self.handoff, self.current(raw), raw)
            self.assertEqual(result['status'], 'room_blocked')

    def test_historical_placement_does_not_remove_its_room_floor(self):
        plan = self.review.unpack()[3]
        point = plan.ordered[0].target
        raw = map_bytes(self.room, {point: {6: 1}})
        source = replace(self.source, map_sha256=hashlib.sha256(raw).hexdigest())
        report = prior_report(self.review, source, prefix=1)
        result = gate.gate_frontier(self.handoff, report, raw)
        self.assertEqual(result['status'], 'ready')
        self.assertEqual(result['room_terrain']['occupied_floor_tiles'], 1)
        raw = map_bytes(self.room, {point: {1: 2, 6: 1}})
        report['source']['map_sha256'] = hashlib.sha256(raw).hexdigest()
        result = gate.gate_frontier(self.handoff, reseal(report), raw)
        self.assertEqual(result['status'], 'room_blocked')

    def test_room_match_never_overrides_route_or_construction_failures(self):
        for status in ('route_blocked', 'route_unestablished', 'construction_pending',
                       'construction_blocked', 'construction_unestablished'):
            result = gate.gate_frontier(self.handoff, prior_report(self.review, self.source, status=status), self.raw)
            self.assertEqual(result['status'], status)
            self.assertEqual(result['execution_frontier_status'], status)

    def test_source_bytes_tick_pause_fortress_and_region_are_exact(self):
        report = prior_report(self.review, self.source)
        for raw in (self.raw[:-1], self.raw + b'x', map_bytes(self.room, tick=11),
                    map_bytes(self.room, folder='other'), map_bytes(self.room, site=8),
                    map_bytes(self.room, paused=False), map_bytes(self.room, size=(31, 32, 1))):
            with self.assertRaises(ValueError):
                gate.gate_frontier(self.handoff, report, raw)
        # Even a rehashed map must obey retained source identity and selection.
        for name, value in (('map_generation', 4), ('inventory_generation', 5),
                            ('endpoint', '127.0.0.1:5001'), ('df_version', 'different')):
            report = prior_report(self.review, replace(self.source, **{name: value}))
            with self.assertRaises(ValueError):
                gate.gate_frontier(self.handoff, report, self.raw)

    def test_changed_parent_review_plan_prefix_and_route_limit_are_refused(self):
        for name, value in (('review_digest', '0' * 64), ('original_plan_digest', '0' * 64),
                            ('original_plan', {}), ('max_route_steps', 1), ('next_step', 'replacement'),
                            ('placed_prefix', [{'slot': 'not-original'}]), ('batch_id', 'bad')):
            report = prior_report(self.review, self.source)
            report[name] = value
            with self.assertRaises(ValueError):
                gate.gate_frontier(self.handoff, reseal(report), self.raw)

    def test_owner_report_policy_and_integrity_are_required(self):
        for name, value in (('policy', 'original-route-constraints-and-fixed-delivery-suffix/2'),
                            ('status', 'complete'), ('all_remaining_selections_checked', False),
                            ('prefix_matches_route_frontier', False), ('schema', 'other')):
            report = prior_report(self.review, self.source)
            report[name] = value
            with self.assertRaises(ValueError):
                gate.gate_frontier(self.handoff, reseal(report), self.raw)
        report = prior_report(self.review, self.source)
        report['evidence_digest'] = '0' * 64
        with self.assertRaises(ValueError):
            gate.gate_frontier(self.handoff, report, self.raw)

    def test_no_work_renewal_and_exact_inclusive_bound(self):
        report = prior_report(self.review, self.source)
        result = gate.gate_frontier(self.handoff, report, self.raw, maximum_work=100_000)
        used = result['work_units']
        exact = gate.gate_frontier(self.handoff, report, self.raw, maximum_work=used)
        self.assertEqual(exact['evidence_digest'], result['evidence_digest'])
        with self.assertRaises(ValueError):
            gate.gate_frontier(self.handoff, report, self.raw, maximum_work=used - 1)
        with self.assertRaises(ValueError):
            gate.gate_frontier(self.handoff, report, self.raw, maximum_work=6_000_001)

    def test_every_authority_checkpoint_including_publication_can_abort(self):
        report = prior_report(self.review, self.source)
        calls = []
        gate.gate_frontier(self.handoff, report, self.raw, lambda: calls.append(1))
        for reject in range(1, len(calls) + 1):
            invoked = 0
            def guard():
                nonlocal invoked
                invoked += 1
                if invoked == reject:
                    raise PermissionError('revoked')
            with self.assertRaises(PermissionError):
                gate.gate_frontier(self.handoff, report, self.raw, guard)

    def test_bounded_hostile_report_before_serialization(self):
        report = prior_report(self.review, self.source)
        report['cycle'] = report
        with self.assertRaises(ValueError):
            gate.gate_frontier(self.handoff, report, self.raw)
        for value in ('x' * (gate.MAX_INPUT + 1), [None] * 65537, float('nan')):
            report = prior_report(self.review, self.source)
            report['bad'] = value
            with self.assertRaises(ValueError):
                gate.gate_frontier(self.handoff, report, self.raw)

    def test_directly_constructed_or_changed_handoff_is_redecoded(self):
        for raw in (self.handoff.raw[:-1], self.handoff.raw + b'\n', b'{}'):
            changed = RoomRouteHandoff(raw, self.handoff.digest)
            with self.assertRaises(ValueError):
                gate.gate_frontier(changed, prior_report(self.review, self.source), self.raw)

    def test_all_rotations_preserve_complete_original_geometry(self):
        for rotation in (0, 90, 180, 270):
            handoff, room, review, source, raw = fixture(rotation=rotation)
            report = prior_report(review, source)
            result = gate.gate_frontier(handoff, report, raw)
            self.assertEqual(result['status'], 'ready')
            self.assertEqual(result['room_terrain']['evaluated_tiles'], len(_requirements(room.json(), lambda: None)))

    def test_full_32_furnishing_parent_and_bounded_deficit_output(self):
        handoff, room, review, source, raw = fixture(count=16, dining=True)
        report = prior_report(review, source, prefix=31)
        result = gate.gate_frontier(handoff, report, raw)
        self.assertEqual(len(result['original_plan']['steps']), 32)
        self.assertEqual(result['status'], 'ready')
        changes = {p: {1: 2} for p in _requirements(room.json(), lambda: None)}
        raw = map_bytes(room, changes)
        report['source']['map_sha256'] = hashlib.sha256(raw).hexdigest()
        result = gate.gate_frontier(handoff, reseal(report), raw)
        deficits = result['room_terrain']['deficits']
        self.assertEqual(len(deficits['rows']), 64)
        self.assertEqual(deficits['count'], len(changes))
        self.assertEqual(deficits['omitted'], len(changes) - 64)
        self.assertLess(len(canonical(result)), gate.MAX_OUTPUT)


if __name__ == '__main__':
    unittest.main()
