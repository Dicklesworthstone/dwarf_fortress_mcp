"""Inventory-to-placement intent tests for df-dfhack-bridge-plane-c-pic.4/.5."""
from __future__ import annotations

from dataclasses import replace
import hashlib
import itertools
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))

from furniture_allocation import Candidate, Request, Slot, allocate
from furniture_handoff import Handoff, InventorySource, MAX_BYTES, MAX_TICK, Selected
from furniture_plan import FurniturePlan, Step, canonical


def source(**changes):
    return replace(InventorySource('127.0.0.1:5000', 17, '51.11', '51.11-r1', 'a' * 64,
                                   800, 50000, (100, 200, 1000)), **changes)


def handoff():
    request = Request('region1', 2, (
        Slot('a-any', 'bed', (15, 15, 2), after=('z-special',), max_distance=10),
        Slot('z-special', 'bed', (16, 15, 2), material=(419, -1), subtype=-1, max_distance=8),
    ), (99,))
    candidates = (Candidate(41, 'bed', (15, 15, 2), (419, -1), -1),
                  Candidate(42, 'bed', (20, 15, 2), (420, 2), 3))
    result = allocate(request, candidates)
    by_id = {c.id: c for c in candidates}
    selected = tuple(Selected(row['slot'], by_id[row['item']], 0) for row in result['assignments'])
    return Handoff(request, source(), selected)


class HandoffTests(unittest.TestCase):
    def test_global_scarcity_assignment_and_original_dependency_order(self):
        h = handoff()
        self.assertEqual([row.candidate.id for row in h.selections], [42, 41])
        self.assertEqual([s.name for s in h.plan().ordered], ['z-special', 'a-any'])
        self.assertEqual(h.plan().steps[0].after, ('z-special',))
        self.assertEqual(FurniturePlan.decode(canonical(h.plan().json())), h.plan())

    def test_round_trip_and_digest_domain(self):
        h = handoff()
        raw = canonical(h.json())
        self.assertEqual(Handoff.decode(raw), h)
        self.assertEqual(h.digest, hashlib.sha256(b'dfmcp-furniture-handoff-python/1\0' + raw).hexdigest())
        self.assertNotEqual(h.digest, h.plan().digest)
        self.assertNotEqual(h.digest, h.request.digest)

    def test_every_input_order_has_identical_plan_and_handoff(self):
        h = handoff()
        for slots, selections in itertools.product(itertools.permutations(h.request.slots),
                                                    itertools.permutations(h.selections)):
            changed = Handoff(replace(h.request, slots=slots), h.source, selections)
            self.assertEqual(changed.digest, h.digest)
            self.assertEqual(canonical(changed.json()), canonical(h.json()))

    def test_returned_objects_cannot_mutate_retained_intent(self):
        h = handoff()
        raw = canonical(h.json())
        returned = h.json()
        returned['request']['slots'].clear()
        returned['selections'][0]['candidate']['material'][0] = 0
        h.compact()['handoff_digest'] = 'b' * 64
        self.assertEqual(canonical(h.json()), raw)
        self.assertEqual(len(h.plan().steps), 2)

    def test_all_source_identity_fields_are_sealed(self):
        h = handoff()
        variants = dict(address='127.0.0.2:5000', generation=18, df_version='52.01',
                        dfhack_version='51.11-r2', capture_sha256='b' * 64,
                        capture_bytes=801, tick=50001, horizons=(101, 201, 1001))
        for field, value in variants.items():
            with self.subTest(field=field):
                changed = replace(h, source=replace(h.source, **{field: value}))
                self.assertNotEqual(h.digest, changed.digest)
                self.assertEqual(changed.plan(), h.plan())

    def test_constraints_and_selected_attributes_are_sealed(self):
        h = handoff()
        for field, value in (('max_distance', 11), ('after', ()), ('material', (420, 2)), ('subtype', 3)):
            slots = (replace(h.request.slots[0], **{field: value}), h.request.slots[1])
            changed = replace(h, request=replace(h.request, slots=slots))
            self.assertNotEqual(h.digest, changed.digest)
        for candidate in (replace(h.selections[0].candidate, id=43),
                          replace(h.selections[0].candidate, position=(21, 15, 2)),
                          replace(h.selections[0].candidate, material=(421, -1)),
                          replace(h.selections[0].candidate, subtype=4)):
            changed = replace(h, selections=(replace(h.selections[0], candidate=candidate), h.selections[1]))
            self.assertNotEqual(h.digest, changed.digest)

    def test_partial_or_duplicate_assignment_never_becomes_executable(self):
        h = handoff()
        variants = ((), h.selections[:1], h.selections + h.selections[:1],
                    (h.selections[0], h.selections[0]),
                    (h.selections[0], replace(h.selections[1], slot='unknown')),
                    (h.selections[0], replace(h.selections[1], candidate=h.selections[0].candidate)))
        for selected in variants:
            with self.subTest(selected=selected), self.assertRaises(ValueError):
                replace(h, selections=selected)

    def test_exclusions_horizon_and_slot_constraints_are_checked_on_import(self):
        h = handoff()
        with self.assertRaises(ValueError):
            replace(h, request=replace(h.request, excluded_items=(41,)))
        with self.assertRaises(ValueError):
            replace(h, source=source(horizons=(100, 200, 42)))
        for kwargs in ({'kind': 'table'}, {'material': (420, -1)}, {'subtype': 0},
                       {'position': (1, 1, 2)}, {'position': (16, 15, 3)}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                item = replace(h.selections[1].candidate, **kwargs)
                replace(h, selections=(h.selections[0], replace(h.selections[1], candidate=item)))

    def test_native_type_mapping_must_remain_bijective(self):
        h = handoff()
        with self.assertRaises(ValueError):
            replace(h, selections=(replace(h.selections[0], native_type=1), h.selections[1]))
        request = replace(h.request, slots=(replace(h.request.slots[0], kind='chair'), h.request.slots[1]))
        row = replace(h.selections[0], candidate=replace(h.selections[0].candidate, kind='chair'))
        with self.assertRaises(ValueError):
            Handoff(request, h.source, (row, h.selections[1]))

    def test_binding_checks_fortress_endpoint_and_software(self):
        h = handoff()
        args = [h.source.address, h.request.folder, h.request.site, h.source.df_version, h.source.dfhack_version]
        h.validate_binding(*args)
        for index, value in enumerate(('127.0.0.1:5001', 'region2', 3, 'other', 'other')):
            changed = args.copy()
            changed[index] = value
            with self.subTest(index=index), self.assertRaises(ValueError):
                h.validate_binding(*changed)
        # Different plugin generation domains do not need coincidentally equal numbers.
        replace(h, source=source(generation=98765)).validate_binding(*args)

    def test_fresh_item_may_move_only_within_original_distance_and_level(self):
        h = handoff()
        step, row = h.plan().steps[0], h.selections[0]
        for position in ((15, 15, 2), (25, 15, 2), (20, 20, 2)):
            h.validate_item(step, replace(row.candidate, position=position), row.native_type, 50001, 101, 201)
        for position in ((26, 15, 2), (15, 15, 3)):
            with self.assertRaises(ValueError):
                h.validate_item(step, replace(row.candidate, position=position), row.native_type, 50001, 101, 201)

    def test_fresh_item_cannot_change_even_unconstrained_original_attributes(self):
        h = handoff()
        step, row = h.plan().steps[0], h.selections[0]
        for kwargs in ({'id': 43}, {'kind': 'chair'}, {'material': (421, -1)}, {'subtype': 4}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                h.validate_item(step, replace(row.candidate, **kwargs), row.native_type, 50000, 100, 200)
        with self.assertRaises(ValueError):
            h.validate_item(step, row.candidate, row.native_type + 1, 50000, 100, 200)

    def test_every_native_selection_component_and_dependency_is_retained(self):
        h = handoff()
        step, row = h.plan().steps[0], h.selections[0]
        for kwargs in ({'name': 'other'}, {'item': 43}, {'kind': 'chair'},
                       {'target': (16, 15, 2)}, {'after': ()}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                h.validate_item(replace(step, **kwargs), row.candidate, row.native_type, 50000, 100, 200)

    def test_time_and_both_effect_identity_horizons_must_not_regress(self):
        h = handoff()
        step, row = h.plan().steps[0], h.selections[0]
        for values in ((49999, 100, 200), (50000, 99, 200), (50000, 100, 199),
                       (True, 100, 200), (50000, False, 200), (MAX_TICK + 1, 100, 200)):
            with self.subTest(values=values), self.assertRaises(ValueError):
                h.validate_item(step, row.candidate, row.native_type, *values)
        h.validate_item(step, row.candidate, row.native_type, MAX_TICK, 2**31 - 1, 2**31 - 1)

    def test_strict_closed_schema_at_every_level(self):
        h = handoff()
        paths = ((), ('request',), ('inventory',), ('selections', 0), ('selections', 0, 'candidate'))
        for path in paths:
            value = h.json()
            obj = value
            for key in path:
                obj = obj[key]
            obj['foreign'] = 1
            with self.subTest(path=path), self.assertRaises(ValueError):
                Handoff.from_json(value)
        value = h.json()
        value['inventory']['profile'] = 'furniture/1.19'
        with self.assertRaises(ValueError):
            Handoff.from_json(value)

    def test_duplicate_fields_noncanonical_bytes_and_nesting_are_refused(self):
        raw = canonical(handoff().json())
        for invalid in (b'', raw + b' ', raw[:-1], b'[' * 11 + b']' * 11,
                        raw.replace(b'"schema":', b'"schema":"duplicate","schema":', 1),
                        b'x' * (MAX_BYTES + 1), b'\xff'):
            with self.subTest(prefix=invalid[:40]), self.assertRaises(ValueError):
                Handoff.decode(invalid)

    def test_hostile_source_types_and_addresses(self):
        fields = dict(address=('localhost:5000', '8.8.8.8:5000', '127.0.0.1:05000', '127.0.0.1:0'),
                      generation=(True, 0, 2**64 - 1), df_version=('', 'x\0y', 'x' * 129),
                      capture_sha256=('A' * 64, 'a' * 63), capture_bytes=(0, 16 * 1024 * 1024 + 1),
                      tick=(-1, MAX_TICK + 1), horizons=([1, 2, 3], (True, 2, 3), (1, 2)))
        for field, values in fields.items():
            for value in values:
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    source(**{field: value})

    def test_dense_32_slot_request_round_trip_fits_persistent_bound(self):
        names = tuple(f'{i:02}-' + 'x' * 30 for i in range(32))
        slots = tuple(Slot(name, 'bed', (i + 2, 15, 2), after=names[max(0, i - 7):i])
                      for i, name in enumerate(names))
        request = Request('r' * 512, 2**31 - 1, slots)
        selected = tuple(Selected(s.name, Candidate(i + 1, 'bed', s.target, (419, -1), -1), 0)
                         for i, s in enumerate(slots))
        h = Handoff(request, source(df_version='v' * 128, dfhack_version='h' * 128), selected)
        raw = canonical(h.json())
        self.assertEqual(Handoff.decode(raw), h)
        self.assertLessEqual(len(raw), MAX_BYTES)
        self.assertEqual(len(h.plan().ordered), 32)
        self.assertFalse(h.compact()['reallocation_permitted'])
        self.assertFalse(h.compact()['items_reserved'])
        self.assertFalse(h.compact()['native_acquisition_independently_attested'])


if __name__ == '__main__':
    unittest.main()
