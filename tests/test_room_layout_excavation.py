"""One-source layout-to-excavation through actual existing component codecs."""
from __future__ import annotations
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

from room_layout_capture_fixtures import ADDRESS, MANIFEST, capture, mask, request, tile
import excavation_observer as native
import map_capture_projection as projection
import room_layout_excavation as compiler
import room_layout as layout
import room_terrain as terrain
from furniture_plan import canonical
from furniture_allocation import Candidate, allocate
from room_excavation_handoff import RoomExcavationHandoff
from room_provisioning import RoomPlan


def solve(value=None, overrides=None, **options):
    value = request() if value is None else value
    raw = canonical(value)
    _, region, _ = compiler.prepare(raw)
    changes = {} if 'access_point' not in value else {tuple(value['access_point']): tile(3)}
    changes.update(overrides or {})
    original = capture(region, changes, **options)
    result, handoff = compiler.solve(raw, original, MANIFEST, ADDRESS)
    return result, handoff, original


class ProjectionTests(unittest.TestCase):
    def test_all_subregions_preserve_exact_native_records_and_unknowns(self):
        parent = native.Region((10, 20, 1), (4, 3, 2))
        overrides = {(10, 20, 1): tile(presence=0), (12, 21, 2): tile(presence=1),
                     (11, 22, 1): tile(8, depth=7, magma=1, dig=7, building=7, units=3)}
        original = capture(parent, overrides, folder='region-\U0001f30d', tick=403201)
        for z in range(1, 3):
            for y in range(20, 23):
                for x in range(10, 14):
                    target = native.Region((x, y, z), (14 - x, 23 - y, 3 - z))
                    out = projection.project(original, MANIFEST, parent, target)
                    self.assertEqual(out.raw, capture(target, overrides, folder='region-\U0001f30d', tick=403201))
                    self.assertEqual(out.parent_sha256, hashlib.sha256(original).hexdigest())
                    self.assertEqual(out.parent_witness, native.decode_capture(original, MANIFEST, parent).witness)
                    self.assertFalse(out.json()['additional_native_read'])
        self.assertEqual(projection.project(original, MANIFEST, parent, parent).raw, original)

    def test_projection_never_extends_or_silently_clamps_source(self):
        parent = native.Region((10, 20, 1), (4, 3, 2))
        raw = capture(parent)
        for target in (native.Region((9, 20, 1), (2, 1, 1)),
                       native.Region((13, 20, 1), (2, 1, 1)),
                       native.Region((10, 20, 0), (1, 1, 1)),
                       native.Region((10, 20, 2), (1, 1, 2))):
            with self.assertRaisesRegex(ValueError, 'beyond'):
                projection.project(raw, MANIFEST, parent, target)

    def test_whole_source_is_validated_even_when_corruption_is_outside_crop(self):
        parent, target = native.Region((10, 10, 1), (3, 3, 2)), native.Region((10, 10, 1), (1, 1, 1))
        raw = capture(parent)
        for broken in (raw[:-1], raw + b'x', raw[:-20] + b'\x03' + raw[-19:], b'X' + raw[1:]):
            with self.assertRaises(ValueError):
                projection.project(broken, MANIFEST, parent, target)
        for end in range(len(raw)):
            with self.assertRaises(ValueError):
                projection.project(raw[:end], MANIFEST, parent, target)

    def test_full_source_identity_is_retained_even_for_unchanged_child(self):
        parent, target = native.Region((10, 10, 1), (3, 3, 2)), native.Region((10, 10, 1), (1, 1, 1))
        left = projection.project(capture(parent), MANIFEST, parent, target)
        right = projection.project(capture(parent, {(12, 12, 2): tile(3)}), MANIFEST, parent, target)
        self.assertEqual(left.raw, right.raw)
        self.assertNotEqual(left.parent_witness, right.parent_witness)
        self.assertNotEqual(left.parent_sha256, right.parent_sha256)
        self.assertEqual(left.witness, right.witness)

    def test_projection_bounds_and_forged_metadata_refuse(self):
        parent = native.Region((1, 1, 1), (1, 1, 1))
        for value in (bytearray(capture(parent)), b'x' * (projection.MAX_BYTES + 1)):
            with self.assertRaises(ValueError):
                projection.project(value, MANIFEST, parent, parent)
        large = native.Region((1, 1, 1), (128, 64, 1))
        with self.assertRaises(ValueError):
            projection.project(capture(large), MANIFEST, large, parent)
        with self.assertRaises(ValueError):
            projection.project(capture(parent), {'generation': 23}, parent, parent)

    def test_projection_cancellation_at_every_guard(self):
        region = native.Region((1, 1, 1), (2, 2, 2))
        raw, calls = capture(region), []
        projection.project(raw, MANIFEST, region, region, lambda: calls.append(1))
        for stop in range(1, len(calls) + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise InterruptedError('cancelled projection')
            with self.assertRaises(InterruptedError):
                projection.project(raw, MANIFEST, region, region, guard)


class ExcavationLayoutTests(unittest.TestCase):
    def test_one_parent_produces_original_plan_and_exact_remaining_dig_handoff(self):
        result, handoff, raw = solve()
        body = result.json()
        self.assertEqual(body['status'], 'excavation_proposed')
        self.assertIsNotNone(handoff)
        self.assertEqual(compiler.replay(result.encode())[0], result)
        self.assertEqual(compiler.replay(result.encode())[1], handoff)
        self.assertEqual(RoomExcavationHandoff.decode(handoff.encode()), handoff)
        plan = handoff.plan()
        self.assertEqual(body['layout']['room_plan'], plan.json())
        self.assertEqual(body['request'], compiler.prepare(canonical(request()))[0])
        self.assertEqual(mask(handoff.json()['remaining_blueprint']),
                         mask(plan.json()['excavation_blueprint']) - {(10, 10, 2)})
        self.assertEqual(body['source']['capture_sha256'], hashlib.sha256(raw).hexdigest())
        self.assertEqual(bytes.fromhex(body['capture_hex']), raw)
        for name in ('layout', 'terrain'):
            self.assertEqual(body['projections'][name]['parent_map_witness'], body['source']['map_witness'])
        self.assertEqual(body['terrain_survey']['source']['game_tick'], body['source']['game_tick'])
        self.assertEqual(body['handoff_digest'], handoff.digest)
        self.assertEqual(plan.request().slots[0].target, (17, 11, 2))

    def test_already_open_tiles_never_enter_the_remaining_mask(self):
        result, handoff, _ = solve(overrides={(16, 10, 2): tile(3), (19, 12, 2): tile(3)})
        remaining = mask(handoff.json()['remaining_blueprint'])
        self.assertFalse(remaining & {(10, 10, 2), (16, 10, 2), (19, 12, 2)})
        self.assertEqual(result.json()['terrain_survey']['target_counts']['observed_floor'], 3)

    def test_vertical_unknown_liquid_or_designation_blocks_fixed_wall_targets(self):
        for bad in (tile(presence=0), tile(presence=1), tile(depth=1), tile(magma=1), tile(dig=1)):
            result, handoff, _ = solve(overrides={(17, 11, 3): bad})
            self.assertEqual(result.json()['status'], 'no_layout_in_domain')
            self.assertIsNone(handoff)
            self.assertIsNone(result.json()['terrain_survey'])
            self.assertGreater(result.json()['halo_constraints']['floor_forbidden_wall_cells'], 0)

    def test_solver_avoids_bad_vertical_context_instead_of_selecting_unsafe_first_site(self):
        value = request(access=False, size=(10, 3, 1), fixed=False)
        result, handoff, _ = solve(value, {(11, 11, 3): tile(presence=1)})
        self.assertEqual(result.json()['status'], 'excavation_proposed')
        selected = handoff.plan().json()['intent']['areas'][0]['origin']
        self.assertGreaterEqual(selected[0], 13)
        self.assertGreater(result.json()['halo_constraints']['floor_forbidden_wall_cells'], 0)

    def test_unrelated_unknown_padding_is_not_an_extra_target_or_global_refusal(self):
        result, handoff, _ = solve(overrides={(10, 15, 1): tile(presence=1)})
        self.assertEqual(result.json()['status'], 'excavation_proposed')
        self.assertNotIn((10, 15, 1), mask(handoff.json()['remaining_blueprint']))

    def test_empty_remaining_work_returns_no_handoff_and_no_completion_claim(self):
        result, handoff, _ = solve(default=tile(3))
        self.assertEqual(result.json()['status'], 'terrain_shapes_satisfied_at_sample')
        self.assertIsNone(handoff)
        self.assertIsNone(result.json()['handoff_digest'])
        self.assertFalse(result.json()['room_completion_proven'])
        self.assertEqual(compiler.replay(result.encode()), (result, None))

    def test_open_floors_do_not_require_mining_halos(self):
        result, handoff, _ = solve(overrides={(17, 11, 3): tile(presence=1)}, default=tile(3))
        self.assertEqual(result.json()['status'], 'terrain_shapes_satisfied_at_sample')
        self.assertIsNone(handoff)

    def test_original_constraints_and_all_furnishings_survive(self):
        value = request()
        value['areas'][0]['item_constraints'] = {'chair': {'material': [7, -1], 'max_distance': 30}}
        value['excluded_items'] = [500, 300]
        before = deepcopy(value)
        result, handoff, _ = solve(value)
        self.assertEqual(value, before)
        desired = handoff.plan().request()
        self.assertEqual(desired.excluded_items, (300, 500))
        self.assertEqual(desired.slots[0].material, (7, -1))
        choices = tuple(Candidate(i, s.kind, s.target, s.material or (0, 1), -1)
                        for i, s in enumerate(desired.slots))
        self.assertEqual(allocate(desired, choices)['status'], 'allocated')
        self.assertEqual(result.json()['layout']['room_plan'], handoff.plan().json())

    def test_complete_rotated_bedroom_keeps_required_wall_ring(self):
        for angle in (0, 90, 180, 270):
            value = request(access=False, size=(20, 20, 1), fixed=False)
            value['areas'] = [{'name': 'sleep', 'rotation': angle,
                'template': {'kind': 'bedroom_cluster', 'rooms_count': 2, 'room_size': [3, 5]}}]
            result, handoff, _ = solve(value)
            self.assertEqual(result.json()['status'], 'excavation_proposed')
            selected = terrain.selection(handoff.plan())
            self.assertFalse(selected.walls & mask(handoff.json()['remaining_blueprint']))
            self.assertEqual(len(handoff.plan().request().slots), 6)

    def test_maximum_eight_area_32_furnishing_request_is_retained_whole(self):
        value = request(access=False, size=(32, 32, 1), fixed=False)
        value['areas'] = [{'name': str(i), 'template': {'kind': 'dining_hall', 'table_count': 2}}
                          for i in range(8)]
        value['search'] = {'max_work': layout.MAX_WORK}
        result, handoff, _ = solve(value)
        self.assertEqual(len(handoff.plan().json()['areas']), 8)
        self.assertEqual(len(handoff.plan().request().slots), 32)
        self.assertLessEqual(len(result.encode()), compiler.MAX_OUTPUT)
        self.assertEqual(compiler.replay(result.encode())[1], handoff)

    def test_source_identity_pause_selection_and_endpoint_refuse(self):
        for options in ({'folder': 'other'}, {'site': 3}, {'paused': False}, {'dimensions': (12, 12, 3)}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                solve(**options)
        value = canonical(request())
        _, parent, _ = compiler.prepare(value)
        wrong = native.Region((9, 9, 1), (12, 7, 3))
        with self.assertRaises(ValueError):
            compiler.solve(value, capture(wrong), MANIFEST, ADDRESS)
        for address in ('10.0.0.1:5000', 'localhost:5000', '127.0.0.1:05000'):
            with self.assertRaises(ValueError):
                compiler.solve(value, capture(parent), MANIFEST, address)

    def test_observed_access_remains_required_before_caller_mask(self):
        value = request()
        value['site_mask'] = {'region': value['region'], 'cells': 'F' + '.' * 59}
        for bad in (tile(), tile(presence=1), tile(3, depth=1), tile(3, units=1), tile(3, building=1)):
            with self.assertRaises(ValueError):
                solve(value, {(10, 10, 2): bad})

    def test_preflight_refuses_unrepresentable_halo_without_clamping(self):
        for size in ((128, 8, 1), (8, 127, 1)):
            with self.assertRaises(ValueError):
                compiler.prepare(canonical(request(access=False, size=size, fixed=False)))
        _, region, _ = compiler.prepare(canonical(request(origin=(1, 1, 1), fixed=False)))
        self.assertEqual(region.origin, (0, 0, 0))

    def test_node_limit_is_inconclusive_and_has_no_handoff(self):
        value = request()
        value['search'] = {'max_nodes': 1}
        result, handoff, _ = solve(value)
        self.assertEqual(result.json()['status'], 'search_limit_reached')
        self.assertFalse(result.json()['layout']['search']['domain_exhausted'])
        self.assertIsNone(handoff)

    def test_aggregate_work_and_cancellation_cover_all_stages(self):
        value = request()
        _, padded, _ = compiler.prepare(canonical(value))
        raw = capture(padded, {(10, 10, 2): tile(3)})
        calls = []
        compiler.solve(canonical(value), raw, MANIFEST, ADDRESS, lambda: calls.append(1))
        for stop in sorted({1, len(calls), *range(2, len(calls), max(1, len(calls) // 20))}):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise InterruptedError('stop whole pipeline')
            with self.assertRaises(InterruptedError):
                compiler.solve(canonical(value), raw, MANIFEST, ADDRESS, guard)
        for maximum in (1, 100, 1000):
            value['search'] = {'max_work': maximum}
            with self.assertRaises(layout.WorkLimit):
                compiler.solve(canonical(value), raw, MANIFEST, ADDRESS)

    def test_rehashed_derived_output_substitutions_fail_full_replay(self):
        result, _, _ = solve()
        for field in ('source', 'projection', 'layout', 'terrain', 'handoff', 'claims', 'halo', 'extra'):
            value = result.json()
            value.pop('compilation_digest')
            if field == 'source': value['source']['game_tick'] += 1
            elif field == 'projection': value['projections']['terrain']['parent_map_witness'] = '0' * 64
            elif field == 'layout': value['layout']['room_plan']['intent']['areas'][0]['origin'][0] -= 1
            elif field == 'terrain': value['terrain_survey']['remaining_wall_targets'].pop()
            elif field == 'handoff': value['handoff_digest'] = '0' * 64
            elif field == 'claims': value['mutation_authority_granted'] = True
            elif field == 'halo': value['halo_constraints']['site_mask']['cells'] = '.' * 60
            else: value['ignore_me'] = True
            # The all-safe fixture's halo string is already all dots. Change a
            # policy instead in that case so every variant actually changes bytes.
            if field == 'halo': value['halo_constraints']['policy'] = 'weakened'
            forged = compiler.ExcavationLayout(canonical(value))
            with self.subTest(field=field), self.assertRaises(ValueError):
                compiler.replay(forged.encode())

    def test_canonical_bytes_strict_bounds_duplicates_and_no_output_alias(self):
        result, _, _ = solve()
        raw = result.encode()
        for broken in (raw + b'\n', raw.replace(b'"map_endpoint":', b'"map_endpoint":"127.0.0.1:5000","map_endpoint":'),
                       b'x' * (compiler.MAX_OUTPUT + 1), b'[' * 17 + b']' * 17):
            with self.assertRaises(ValueError): compiler.replay(broken)
        value = result.json(); value['layout']['room_plan']['intent']['areas'].clear()
        self.assertEqual(result.encode(), raw)

    def test_process_replay_is_deterministic_across_hash_seeds(self):
        result, _, _ = solve()
        code = 'import sys,room_layout_excavation as c;sys.stdout.buffer.write(c.replay(sys.stdin.buffer.read())[0].encode())'
        root = Path(__file__).resolve().parents[1]
        for seed in ('1', '13', 'random'):
            child = subprocess.run([sys.executable, '-c', code], input=result.encode(), capture_output=True,
                env={**os.environ, 'PYTHONPATH': str(root / 'scripts'), 'PYTHONHASHSEED': seed}, timeout=15, check=True)
            self.assertEqual(child.stdout, result.encode())

    def test_no_mutation_acquisition_or_completion_authority_is_manufactured(self):
        result, _, _ = solve()
        for field in ('additional_native_read', 'native_acquisition_independently_attested',
                      'native_excavation_eligibility_proven', 'room_completion_proven',
                      'existing_effects_reconciled', 'replacement_effect_key_authorized',
                      'mutation_authority_granted', 'game_effect_performed', 'production_admitted'):
            self.assertIs(result.json()[field], False)
        self.assertTrue(result.json()['single_source_capture'])


if __name__ == '__main__':
    unittest.main()
