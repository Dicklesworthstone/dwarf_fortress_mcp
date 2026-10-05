"""Complete room intent -> one decoded terrain capture -> exact residual proposal."""
from __future__ import annotations

from dataclasses import replace
import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))

import excavation_observer as e
import room_terrain as t
from furniture_plan import canonical
from room_provisioning import RoomPlan
from room_terrain_fixtures import MANIFEST, bedroom, capture, coordinates, dining, expand, plan, request, visible


class RoomTerrainTests(unittest.TestCase):
    def check_false_claims(self, result):
        for field in ('mutation_authority_granted', 'native_excavation_eligibility_proven',
                      'existing_effects_reconciled', 'replacement_effect_key_authorized',
                      'current_terrain_proven', 'continuous_wall_preservation_proven',
                      'structural_safety_proven', 'native_pathfinding_proven',
                      'room_assignments_created', 'room_completion_proven',
                      'construction_completion_proven', 'production_admitted',
                      'residual_completion_is_room_completion'):
            self.assertIs(result[field], False, field)

    def test_complete_partial_and_already_dug_rooms(self):
        original = plan()
        selected = t.selection(original)
        for completed in (set(), set(sorted(selected.floors)[::3]), set(selected.floors)):
            with self.subTest(completed=len(completed)):
                raw = capture(selected.region, {p: visible(3) for p in completed})
                result = t.survey(original, raw, MANIFEST)
                pending = selected.floors - completed
                self.assertEqual(set(map(tuple, result['remaining_wall_targets'])), pending)
                self.assertEqual(result['target_counts']['observed_floor'], len(completed))
                self.assertEqual(result['blockers']['count'], 0)
                self.assertEqual(result['room_plan'], original.json())
                self.assertEqual(result['source']['capture_sha256'], hashlib.sha256(raw).hexdigest())
                if pending:
                    self.assertEqual(result['status'], 'excavation_proposed')
                    self.assertEqual(expand(result['remaining_blueprint']), pending)
                    self.assertFalse(pending & selected.walls)
                    self.assertFalse(result['terrain_shapes_satisfied_at_sample'])
                else:
                    self.assertEqual(result['status'], 'terrain_shapes_satisfied_at_sample')
                    self.assertIsNone(result['remaining_blueprint'])
                    self.assertIsNone(result['remaining_mask_digest'])
                    self.assertTrue(result['terrain_shapes_satisfied_at_sample'])
                self.check_false_claims(result)

    def test_all_4096_small_masks_have_an_exact_disjoint_cover(self):
        grid = [(x + 10, y + 10, 2) for y in range(3) for x in range(4)]
        for bits in range(4096):
            mask = frozenset(p for i, p in enumerate(grid) if bits & (1 << i))
            for span in (8, 128):
                parts = t.partition(mask, span)
                self.assertEqual(expand({'parts': parts}), mask)
                self.assertEqual(parts, t.partition(frozenset(reversed(sorted(mask))), span))
                self.assertTrue(all(max(p['region']['size']) <= span for p in parts))

    def test_all_room_template_counts_and_dimensions(self):
        accepted = refused = 0
        for count in range(1, 11):
            for w in range(3, 8):
                for h in range(3, 8):
                    try:
                        original = plan(bedroom(count, (w, h)))
                    except ValueError:
                        refused += 1
                        continue
                    selected = t.selection(original)
                    result = t.survey(original, capture(selected.region), MANIFEST)
                    self.assertEqual(result['remaining_mask_digest'], original.json()['excavation_mask_digest'])
                    self.assertEqual(expand(result['remaining_blueprint']), selected.floors)
                    self.assertFalse(selected.floors & selected.walls)
                    self.assertEqual(result['target_counts']['remaining_wall'], len(selected.floors))
                    accepted += 1
        self.assertEqual(accepted + refused, 250)
        self.assertGreater(accepted, 200)
        for count in range(1, 17):
            for columns in range(1, 5):
                original = plan(dining(count, columns))
                selected = t.selection(original)
                result = t.survey(original, capture(selected.region), MANIFEST)
                self.assertEqual(expand(result['remaining_blueprint']), selected.floors)
                self.assertEqual(result['coverage']['required_wall_tiles'], 0)

    def test_target_failures_never_emit_a_successful_subset(self):
        original = plan()
        selected = t.selection(original)
        target = min(selected.floors)
        failures = [b'\x00', b'\x01', visible(3, depth=1), visible(2, magma=1),
                    visible(2, dig=1), visible(3, dig=1), visible(2, building=1), visible(2, units=1)]
        failures += [visible(shape) for shape in (0, 1, 4, 5, 6, 7, 8)]
        for tile in failures:
            with self.subTest(tile=tile):
                result = t.survey(original, capture(selected.region, {target: tile}), MANIFEST)
                self.assertEqual(result['status'], 'blocked')
                self.assertIsNone(result['remaining_blueprint'])
                self.assertTrue(any(row['domain'] == 'target' for row in result['blockers']['rows']))
                self.assertEqual(sum(result['target_counts'].values()), len(selected.floors))
                self.check_false_claims(result)

    def test_required_walls_cannot_be_ignored_filled_or_excavated(self):
        original = plan()
        selected = t.selection(original)
        wall = min(selected.walls)
        done = {p: visible(3) for p in selected.floors}
        for tile in (b'\x00', b'\x01', visible(3), visible(2, dig=1), visible(2, depth=1),
                     visible(2, magma=1), visible(2, building=1), visible(2, units=1)):
            result = t.survey(original, capture(selected.region, {**done, wall: tile}), MANIFEST)
            self.assertEqual(result['status'], 'blocked')
            self.assertIsNone(result['remaining_blueprint'])
            self.assertEqual(result['remaining_wall_targets'], [])
            self.assertFalse(result['terrain_shapes_satisfied_at_sample'])
            self.assertEqual(result['blockers']['rows'][0]['domain'], 'required_wall')

    def test_vertical_and_diagonal_remaining_halo_is_not_assumed_clear(self):
        original = plan()
        selected = t.selection(original)
        x, y, z = min(selected.floors)
        for point in ((x, y, z - 1), (x + 1, y + 1, z + 1)):
            for tile in (b'\x00', b'\x01', visible(3, depth=1), visible(2, magma=1), visible(2, dig=1)):
                result = t.survey(original, capture(selected.region, {point: tile}), MANIFEST)
                self.assertEqual(result['status'], 'blocked')
                self.assertIsNone(result['remaining_blueprint'])
                self.assertTrue(any(row['domain'] == 'remaining_halo' and row['coordinate'] == list(point)
                                    for row in result['blockers']['rows']))

    def test_capture_holes_are_not_targets_or_required_halo(self):
        original = plan(dining(1, 1, origin=(10, 10, 2)), dining(1, 1, origin=(30, 10, 2), name='other'))
        selected = t.selection(original)
        hole = (22, 11, 2)
        self.assertIn(hole, coordinates(selected.region))
        self.assertNotIn(hole, selected.floors)
        result = t.survey(original, capture(selected.region, {hole: b'\x01'}), MANIFEST)
        self.assertEqual(result['status'], 'excavation_proposed')
        self.assertEqual(expand(result['remaining_blueprint']), selected.floors)
        self.assertGreater(result['coverage']['unused_capture_tiles'], 0)
        # With no remaining walls, unknown unused vertical context is not a floor
        # deficit. It is still not structural safety or continuous preservation.
        done = {p: visible(3) for p in selected.floors}
        done[selected.region.origin] = b'\x01'
        result = t.survey(original, capture(selected.region, done), MANIFEST)
        self.assertTrue(result['terrain_shapes_satisfied_at_sample'])
        self.check_false_claims(result)

    def test_multilevel_exact_mask_and_original_constraints_survive(self):
        value = request(bedroom(1, origin=(10, 10, 2)), dining(2, 2, origin=(10, 10, 4)))
        value['areas'][0]['item_constraints'] = {'bed': {'material': [419, -1], 'subtype': 7, 'max_distance': 10}}
        value['excluded_items'] = [91, 17]
        value['excluded_regions'] = [{'origin': [40, 40, 2], 'size': [100, 100, 100]}]
        original = RoomPlan.compile(value)
        selected = t.selection(original)
        completed = {p for p in selected.floors if p[2] == 2}
        result = t.survey(original, capture(selected.region, {p: visible(3) for p in completed}), MANIFEST)
        self.assertEqual(expand(result['remaining_blueprint']), selected.floors - completed)
        self.assertEqual(result['room_plan'], original.json())
        self.assertNotIn(3, {p[2] for p in expand(result['remaining_blueprint'])})
        self.assertEqual(result['completion_goal_location'], 'room_plan.excavation_blueprint')

    def test_fragmented_residual_exhaustion_returns_no_partial_artifact(self):
        original = plan(bedroom(8, (7, 7)))
        selected = t.selection(original)
        done = {p: visible(3) for p in selected.floors if (p[0] + p[1]) % 2}
        result = t.survey(original, capture(selected.region, done), MANIFEST)
        self.assertEqual(result['status'], 'blocked')
        self.assertIsNone(result['remaining_blueprint'])
        self.assertGreater(result['partition']['blueprint_parts'], 32)
        self.assertGreater(result['partition']['normal_mining_rectangles'], 128)
        self.assertEqual(set(map(tuple, result['remaining_wall_targets'])), selected.floors - done.keys())
        self.assertIn('capacity.residual_exceeds_32_blueprint_parts', result['blockers']['counts'])

    def test_complete_blocker_accounting_and_bounded_diagnostics(self):
        original = plan(bedroom(8, (7, 7)))
        selected = t.selection(original)
        result = t.survey(original, capture(selected.region, {p: b'\x01' for p in selected.floors}), MANIFEST)
        self.assertEqual(result['blockers']['count'], len(selected.floors))
        self.assertEqual(result['blockers']['shown'], 128)
        self.assertEqual(result['blockers']['omitted'], len(selected.floors) - 128)
        self.assertEqual(result['remaining_wall_targets'], [])
        self.assertIsNone(result['remaining_blueprint'])

    def test_room_artifact_is_rebuilt_before_using_geometry_or_claims(self):
        original = plan()
        for field, value in (('target_tiles', 1), ('room_completion_proven', True), ('policy', 'other')):
            body = json.loads(original._body)
            body[field] = value
            corrupted = RoomPlan(canonical(body))
            with self.assertRaises(ValueError):
                t.selection(corrupted)
        body = json.loads(original._body)
        body['excavation_blueprint']['parts'][0]['region']['origin'][0] += 1
        with self.assertRaises(ValueError):
            t.selection(RoomPlan(canonical(body)))

    def test_capture_selection_source_extent_and_manifest_are_validated(self):
        original = plan()
        selected = t.selection(original)
        for kwargs in ({'folder': 'foreign'}, {'site': 3}, {'dimensions': (10, 10, 1)}):
            with self.assertRaises(ValueError):
                t.survey(original, capture(selected.region, **kwargs), MANIFEST)
        other = e.Region(selected.region.origin, (selected.region.size[0] - 1, *selected.region.size[1:]))
        with self.assertRaises(ValueError):
            t.survey(original, capture(other), MANIFEST)
        raw = capture(selected.region)
        for bad in (raw[:-1], raw + b'\0', b'X' + raw[1:], bytes(t.MAX_CAPTURE_BYTES + 1)):
            with self.assertRaises(ValueError):
                t.survey(original, bad, MANIFEST)
        bad_manifest = replace(MANIFEST)
        object.__setattr__(bad_manifest, 'generation', 0)
        with self.assertRaises(ValueError):
            t.survey(original, raw, bad_manifest)
        self.assertNotEqual(t.survey(original, raw, MANIFEST)['survey_digest'],
                            t.survey(original, raw, replace(MANIFEST, generation=8))['survey_digest'])

    def test_map_edges_and_padded_axis_overflow_are_not_clamped(self):
        for origin in ((3, 1, 1), (32764, 32762, 32766)):
            original = plan(bedroom(1, origin=origin))
            selected = t.selection(original)
            result = t.survey(original, capture(selected.region), MANIFEST)
            self.assertEqual(result['status'], 'excavation_proposed')
        # A valid unpadded room recipe can be too wide for one native padded read.
        original = plan(dining(1, 1, origin=(1, 10, 2)), dining(1, 1, origin=(125, 10, 2), name='far'))
        self.assertEqual(original.json()['capture_region']['size'][0], 128)
        with self.assertRaises(ValueError):
            t.selection(original)

    def test_occupied_floors_are_geometry_not_furnishing_permission(self):
        original = plan()
        selected = t.selection(original)
        result = t.survey(original, capture(selected.region, {p: visible(3, building=1, units=1)
                           for p in selected.floors}, paused=False), MANIFEST)
        self.assertTrue(result['terrain_shapes_satisfied_at_sample'])
        self.assertEqual(result['occupied_floor_tiles'], len(selected.floors))
        self.assertFalse(result['source']['paused_at_capture'])
        self.check_false_claims(result)

    def test_determinism_full_intent_binding_and_nonaliasing(self):
        areas = [bedroom(1), dining(2, 2, origin=(10, 18, 2))]
        first, second = plan(*areas), plan(*reversed(areas))
        selected = t.selection(first)
        raw = capture(selected.region)
        a, b = t.survey(first, raw, MANIFEST), t.survey(second, raw, MANIFEST)
        self.assertEqual(canonical(a), canonical(b))
        digest = a.pop('survey_digest')
        self.assertEqual(digest, hashlib.sha256(b'dfmcp-room-terrain-survey/1\0' + canonical(a)).hexdigest())
        a['room_plan']['intent']['areas'].clear()
        self.assertEqual(t.survey(first, raw, MANIFEST), b)
        self.assertLess(len(canonical(b)), t.MAX_OUTPUT - 4096)

    def test_guard_interruptions_cannot_publish_a_partial_proposal(self):
        original = plan(dining(1, 1))
        selected = t.selection(original)
        raw = capture(selected.region)
        calls = 0
        def count():
            nonlocal calls
            calls += 1
        t.survey(original, raw, MANIFEST, count)
        # Every guarded boundary, including the final post-serialization guard.
        for stop in range(1, calls + 1):
            current = 0
            def interrupt():
                nonlocal current
                current += 1
                if current == stop:
                    raise InterruptedError('injected boundary')
            with self.assertRaises(InterruptedError):
                t.survey(original, raw, MANIFEST, interrupt)


if __name__ == '__main__':
    unittest.main()
