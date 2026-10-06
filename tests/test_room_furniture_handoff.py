"""Real room compiler/allocator/handoff semantics; no native game claims."""
from dataclasses import replace
import copy
import json
import unittest
from unittest.mock import patch

from furniture_allocation import Candidate, Request, allocate
from furniture_handoff import Handoff, InventorySource, Selected
from furniture_plan import FurniturePlan, canonical
from room_provisioning import RoomPlan
from room_furniture_handoff import MAX_BYTES, MAX_DEPTH, RoomFurnitureHandoff


def room(*, height=3, count=2, dining=False, excluded=()):
    kinds = ('chair', 'table') if dining else ('bed', 'chair', 'table')
    template = ({'kind': 'dining_hall', 'table_count': count, 'columns': 4} if dining else
                {'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': [3, height]})
    return RoomPlan.compile({'schema': 'dfmcp.room-provisioning-request/1',
        'world_folder': 'region1', 'site': 2, 'excluded_items': list(excluded),
        'excluded_regions': [{'origin': [0, 0, 0], 'size': [1, 1, 1]}],
        'areas': [{'name': 'rooms', 'origin': [10, 10, 2], 'template': template,
                   'item_constraints': {k: {'material': [419, -1], 'subtype': -1,
                                           'max_distance': 50} for k in kinds}}]})


def allocated(plan, *, address='127.0.0.1:5000', generation=7, tick=100):
    request = plan.request()
    candidates = tuple(Candidate(100 + i, slot.kind, slot.target, (419, -1), -1)
                       for i, slot in enumerate(request.slots))
    report = allocate(request, candidates)
    if report['status'] != 'allocated':
        raise AssertionError(report)
    by_id = {c.id: c for c in candidates}
    source = InventorySource(address, generation, 'test-df', 'test-dfhack', 'a' * 64,
                             4096, tick, (200, 300, 1000))
    handoff = Handoff(request, source, tuple(Selected(row['slot'], by_id[row['item']],
                           {'bed': 101, 'chair': 102, 'table': 103}[row['kind']])
                           for row in report['assignments']))
    return RoomFurnitureHandoff(plan, handoff)


class RoomFurnitureHandoffTests(unittest.TestCase):
    def setUp(self):
        self.room = room()
        self.value = allocated(self.room)

    def test_round_trip_retains_all_original_intent_and_exact_allocated_plan(self):
        restored = RoomFurnitureHandoff.decode(self.value.encode())
        self.assertEqual(restored, self.value)
        self.assertEqual(restored.room_plan.encode(), self.room.encode())
        self.assertEqual(restored.allocation.request, self.room.request())
        self.assertEqual(restored.allocation.plan().ordered, self.value.allocation.plan().ordered)
        self.assertEqual(restored.encode(), canonical(restored.json()))
        self.assertFalse(restored.compact()['room_completion_proven'])
        self.assertFalse(restored.compact()['items_reserved'])

    def test_same_furniture_but_different_room_geometry_has_different_identity(self):
        taller = room(height=4)
        self.assertEqual(taller.request(), self.room.request())
        other = RoomFurnitureHandoff(taller, self.value.allocation)
        self.assertEqual(other.allocation.digest, self.value.allocation.digest)
        self.assertNotEqual(other.digest, self.value.digest)
        self.assertNotEqual(other.room_plan.digest, self.value.room_plan.digest)

    def test_excluded_regions_bind_identity_even_when_furniture_is_unchanged(self):
        request = self.room.json()['intent']
        request['excluded_regions'][0]['size'] = [2, 2, 1]
        other = RoomFurnitureHandoff(RoomPlan.compile(request), self.value.allocation)
        self.assertEqual(other.allocation.digest, self.value.allocation.digest)
        self.assertNotEqual(other.digest, self.value.digest)

    def test_inventory_identity_and_native_generation_bind_composite(self):
        for change in ({'generation': 8}, {'tick': 101}, {'address': '127.0.0.1:5001'}):
            self.assertNotEqual(allocated(self.room, **change).digest, self.value.digest)

    def test_room_or_request_substitution_is_rejected_not_repaired(self):
        for field, value in [('max_distance', 51), ('subtype', None), ('material', None),
                             ('target', [11, 10, 2]), ('after', [])]:
            with self.subTest(field=field):
                raw = self.value.json()
                # Use a dependent slot so dropping dependencies changes intent.
                raw['allocation']['request']['slots'][-1][field] = value
                self.assertRaises(ValueError, RoomFurnitureHandoff.from_json, raw)
        for field, value in [('world_folder', 'other'), ('site', 3), ('excluded_items', [999])]:
            raw = self.value.json()
            raw['allocation']['request'][field] = value
            self.assertRaises(ValueError, RoomFurnitureHandoff.from_json, raw)

    def test_valid_smaller_allocation_cannot_replace_the_original_room_set(self):
        request = self.value.allocation.request.json()
        request['slots'] = request['slots'][:3]
        smaller = Handoff(Request.from_json(request), self.value.allocation.source,
                          self.value.allocation.selections[:3])
        self.assertRaises(ValueError, RoomFurnitureHandoff, self.room, smaller)

    def test_rehashed_forged_room_projection_is_recomputed(self):
        for field in ('areas', 'excavation_blueprint', 'capture_region'):
            raw = self.value.json()
            raw['room_plan'][field] = []
            raw['room_plan'].pop('plan_digest')
            forged = RoomPlan(canonical(raw['room_plan']))
            self.assertRaises(ValueError, RoomFurnitureHandoff, forged, self.value.allocation)
        forged = copy.deepcopy(self.value.allocation)
        object.__setattr__(forged.selections[0].candidate, 'material', (999, 0))
        self.assertRaises(ValueError, RoomFurnitureHandoff, self.room, forged)

    def test_closed_canonical_wire_and_bounds(self):
        raw = self.value.encode()
        for candidate in (raw + b'\n', b' ' + raw, raw[:-1], b'{}', b'[]',
                          b'{"schema":"x","schema":"y"}', b'{"x":NaN}',
                          b'[' * (MAX_DEPTH + 1) + b']' * (MAX_DEPTH + 1),
                          b' ' * (MAX_BYTES + 1)):
            self.assertRaises(ValueError, RoomFurnitureHandoff.decode, candidate)
        for key in ('schema', 'room_plan', 'allocation'):
            value = self.value.json()
            del value[key]
            self.assertRaises(ValueError, RoomFurnitureHandoff.from_json, value)
        value = self.value.json()
        value['permission'] = True
        self.assertRaises(ValueError, RoomFurnitureHandoff.from_json, value)

    def test_malformed_json_is_rejected_before_general_parser_when_over_budget(self):
        with patch('room_furniture_handoff.json.loads', side_effect=AssertionError('parser reached')):
            self.assertRaises(ValueError, RoomFurnitureHandoff.decode, b'x' * (MAX_BYTES + 1))
            self.assertRaises(ValueError, RoomFurnitureHandoff.decode, b'[' * (MAX_DEPTH + 1))

    def test_32_slots_and_646_exclusions_survive_round_trip(self):
        plan = room(dining=True, count=16, excluded=range(10000, 10646))
        value = allocated(plan)
        restored = RoomFurnitureHandoff.decode(value.encode())
        self.assertEqual(len(restored.allocation.selections), 32)
        self.assertEqual(restored.room_plan.request().excluded_items, tuple(range(10000, 10646)))
        self.assertLessEqual(len(value.encode()), MAX_BYTES)
        restored.check_dimensions([32768, 32768, 32768])

    def test_complete_corridor_and_room_walls_must_fit_not_only_furniture(self):
        self.value.check_dimensions([32768, 32768, 32768])
        # All furniture targets and their 3x3 halos fit; the south corridor does not.
        self.value.allocation.plan().check_dimensions([30, 13, 3])
        self.assertRaises(ValueError, self.value.check_dimensions, [30, 13, 3])
        # The east bedroom wall must fit even if target-only validation is skipped.
        with patch.object(FurniturePlan, 'check_dimensions', return_value=None):
            self.assertRaises(ValueError, self.value.check_dimensions, [17, 30, 3])
        for dimensions in ([True, 30, 3], [30, 30], [30, 30, 0], [32769, 30, 3]):
            self.assertRaises(ValueError, self.value.check_dimensions, dimensions)

    def test_original_native_item_and_source_constraints_still_apply(self):
        h = self.value.allocation
        h.validate_binding(h.source.address, 'region1', 2, 'test-df', 'test-dfhack')
        step = h.plan().steps[0]
        selected = h.selections[0]
        h.validate_item(step, selected.candidate, selected.native_type, 100, 200, 300)
        for change in ({'position': (100, 100, 2)}, {'position': (10, 10, 3)},
                       {'material': (999, 0)}, {'subtype': 2}):
            self.assertRaises(ValueError, h.validate_item, step,
                              replace(selected.candidate, **change), selected.native_type, 100, 200, 300)
        self.assertRaises(ValueError, h.validate_item, step, selected.candidate,
                          selected.native_type, 99, 200, 300)

    def test_public_views_cannot_mutate_the_retained_plan(self):
        original = self.value.encode()
        view = self.value.json()
        view['room_plan']['intent']['areas'].clear()
        view['allocation']['selections'].clear()
        self.assertEqual(self.value.encode(), original)

    def test_guard_abort_at_every_decode_boundary_returns_no_handoff(self):
        calls = []
        RoomFurnitureHandoff.decode(self.value.encode(), lambda: calls.append(1))
        class Stop(Exception):
            pass
        for boundary in range(len(calls)):
            left = [boundary]
            def guard():
                if not left[0]:
                    raise Stop()
                left[0] -= 1
            self.assertRaises(Stop, RoomFurnitureHandoff.decode, self.value.encode(), guard)


if __name__ == '__main__':
    unittest.main()
