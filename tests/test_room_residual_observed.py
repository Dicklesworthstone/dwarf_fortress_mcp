"""Raw-source residual proof and complete original excavation handoff replay."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path[:0] = [str(Path(__file__).resolve().parents[1] / 'scripts')]
import excavation_observer as native
import room_layout as layout
import room_layout_observed as observed
import room_layout_excavation as compilation
import room_terrain as terrain
from furniture_plan import canonical
from room_excavation_handoff import RoomExcavationHandoff
from test_room_residual_capacity import request, capture, points, cost_at

MANIFEST = native.Manifest(3, 'fixture', 'fixture')


def observed_request(*, small=False, restricted=True):
    value = request(small=small, restricted=restricted)
    value.pop('excavation_costs')
    value['objective'] = observed.EXCAVATION_OBJECTIVE
    return value


def source_request(value):
    # Test-only synthetic source, independent of caller site-use constraints.
    result = deepcopy(value)
    x0, y0, _ = value['region']['origin']
    width, height, _ = value['region']['size']
    result['excavation_costs'] = ''.join('1' if x >= 23 or (x+y)%2 == 0 else '0'
        for y in range(y0, y0+height) for x in range(x0, x0+width))
    return result


def compiled(value, *, cells=None):
    source = source_request(value)
    if cells is not None:
        source['excavation_costs'] = cells
    _, region, _ = compilation.prepare(canonical(value))
    return compilation.solve(canonical(value), capture(source, region), MANIFEST, '127.0.0.1:5000')


class ObservedResidualTests(unittest.TestCase):
    def test_padded_pipeline_changes_blocked_result_into_whole_valid_handoff(self):
        old, handoff = compiled(observed_request(restricted=False))
        self.assertEqual(old.json()['status'], 'blocked')
        self.assertIsNone(handoff)
        self.assertEqual(old.json()['terrain_survey']['partition']['blueprint_parts'], 59)
        result, handoff = compiled(observed_request())
        self.assertEqual(result.json()['status'], 'excavation_proposed')
        self.assertIsNotNone(handoff)
        self.assertEqual(len(handoff.plan().request().slots), 32)
        self.assertEqual(result.json()['terrain_survey']['partition']['blueprint_parts'], 28)
        self.assertEqual(RoomExcavationHandoff.decode(handoff.encode()), handoff)
        self.assertEqual(compilation.replay(result.encode()), (result, handoff))

    def test_raw_wall_count_digest_and_partition_agree_at_all_three_layers(self):
        result, handoff = compiled(observed_request())
        body = result.json()
        assumed = body['layout']['residual_partition']['selected']
        evidence = body['layout_evidence']['residual_capacity_evidence']
        survey = body['terrain_survey']
        self.assertEqual(assumed['remaining_mask_digest'], evidence['remaining_mask_digest'])
        self.assertEqual(evidence['remaining_mask_digest'], survey['remaining_mask_digest'])
        self.assertEqual(survey['remaining_mask_digest'], handoff.json()['remaining_mask_digest'])
        self.assertEqual(evidence['remaining_wall_tiles'], 90)
        self.assertEqual(evidence['blueprint_parts'], 28)
        self.assertEqual(evidence['normal_mining_rectangles'], 28)
        self.assertTrue(body['single_source_capture'])
        self.assertFalse(body['additional_native_read'])
        self.assertEqual(body['projections']['layout']['parent_map_witness'], body['source']['map_witness'])
        self.assertEqual(evidence['map_witness'], body['projections']['layout']['map_witness'])

    def test_caller_floor_permissions_do_not_turn_fragmented_walls_into_free_cells(self):
        value = observed_request()
        value['site_mask'] = {'region': value['region'], 'cells': 'F'*252}
        result, handoff = compiled(value)
        self.assertEqual(result.json()['layout']['room_plan']['intent']['areas'][0]['origin'], [17, 10, 2])
        self.assertEqual(result.json()['layout']['residual_partition']['capacity_refusals'], 7)
        self.assertEqual(result.json()['terrain_survey']['target_counts']['remaining_wall'], 90)
        self.assertIsNotNone(handoff)

    def test_observed_prepare_refuses_imported_masks_and_unrelated_objectives(self):
        for field, value in (('excavation_costs', '0'*252), ('residual_partition', True),
                             ('residual_partition', 'other'), ('objective', layout.MINIMUM_TARGETS_OBJECTIVE)):
            doc = observed_request()
            doc[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                observed.prepare(canonical(doc))

    def test_no_capacity_valid_position_means_no_partial_residual(self):
        value = observed_request()
        value['areas'][0]['origin'] = [10, 10, 2]
        result, handoff = compiled(value)
        self.assertEqual(result.json()['status'], 'no_layout_in_domain')
        self.assertIsNone(handoff)
        self.assertIsNone(result.json()['terrain_survey'])
        self.assertIsNone(result.json()['layout_evidence']['residual_capacity_evidence']['remaining_wall_tiles'])
        self.assertEqual(compilation.replay(result.encode()), (result, None))

    def test_node_limited_handoff_is_complete_but_never_optimal(self):
        value = observed_request()
        value['search'] = {'max_nodes': 9}
        result, handoff = compiled(value)
        self.assertIsNotNone(handoff)
        self.assertEqual(len(handoff.plan().request().slots), 32)
        self.assertFalse(result.json()['layout']['optimization']['minimum_in_compiler_domain_proven'])
        self.assertTrue(result.json()['layout']['residual_partition']['selected']['within_profile_limits'])

    def test_empty_residual_is_not_misreported_as_absence_or_a_dig_handoff(self):
        result, handoff = compiled(observed_request(), cells='0'*252)
        self.assertEqual(result.json()['status'], 'terrain_shapes_satisfied_at_sample')
        self.assertIsNone(handoff)
        evidence = result.json()['layout_evidence']['residual_capacity_evidence']
        self.assertEqual(evidence['remaining_wall_tiles'], 0)
        self.assertEqual(evidence['blueprint_parts'], 0)
        self.assertTrue(evidence['within_profile_limits'])
        self.assertEqual(compilation.replay(result.encode()), (result, None))

    def test_modified_capacity_and_request_fail_even_after_resigning_compilation(self):
        result, _ = compiled(observed_request())
        for mode in ('counts', 'strategy', 'source'):
            body = result.json()
            body.pop('compilation_digest')
            if mode == 'counts':
                body['layout_evidence']['residual_capacity_evidence']['blueprint_parts'] = 1
            elif mode == 'strategy':
                body['request'].pop('residual_partition')
            else:
                body['source']['manifest']['generation'] += 1
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                compilation.replay(compilation.ExcavationLayout(canonical(body)).encode())

    def test_remaining_handoff_contains_every_and_only_observed_wall_target(self):
        value = observed_request()
        result, handoff = compiled(value)
        all_targets = set(points(handoff.plan().json()['excavation_blueprint']['parts']))
        expected = {p for p in all_targets if cost_at(source_request(value), p)}
        raw_parts = handoff.json()['remaining_blueprint']['parts']
        actual = points(raw_parts)
        self.assertEqual(set(actual), expected)
        self.assertEqual(len(actual), len(expected))
        self.assertEqual(len(actual), 90)
        self.assertFalse(handoff.summary()['replacement_effect_key_authorized'])
        self.assertFalse(result.json()['native_excavation_eligibility_proven'])

    def test_original_input_is_not_mutated_and_no_custody_is_created(self):
        value = observed_request()
        before = deepcopy(value)
        result, _ = compiled(value)
        self.assertEqual(value, before)
        for flag in ('mutation_authority_granted', 'game_effect_performed', 'production_admitted',
                     'existing_effects_reconciled', 'replacement_effect_key_authorized'):
            self.assertFalse(result.json()[flag])

    def test_eight_original_areas_and_all_32_furnishings_survive_handoff(self):
        value = observed_request()
        value['region']['size'] = [32, 32, 1]
        value['areas'] = [
            {'name': f'hall{i}', 'origin': [10 + 8*(i % 4), 10 + 4*(i // 4), 2],
             'template': {'kind': 'dining_hall', 'table_count': 2, 'columns': 2}}
            for i in range(8)]
        result, handoff = compiled(value, cells='1'*1024)
        self.assertEqual(result.json()['status'], 'excavation_proposed')
        self.assertIsNotNone(handoff)
        self.assertEqual(len(handoff.plan().json()['areas']), 8)
        self.assertEqual(len(handoff.plan().request().slots), 32)
        capacity = result.json()['layout']['residual_partition']['selected']
        self.assertTrue(capacity['within_profile_limits'])
        self.assertEqual(capacity['remaining_tiles'], handoff.plan().json()['target_tiles'])
        self.assertEqual(RoomExcavationHandoff.decode(handoff.encode()), handoff)

    def test_original_bedroom_wall_ring_is_still_a_hard_constraint(self):
        value = observed_request(small=True)
        value['region']['size'] = [9, 8, 1]
        value['areas'] = [{'name': 'bedroom', 'origin': [12, 11, 2],
                          'template': {'kind': 'bedroom_cluster', 'rooms_count': 1, 'room_size': [3, 3]}}]
        # All existing floors cannot be used as preserved bedroom walls.
        result, handoff = compiled(value, cells='0'*72)
        self.assertEqual(result.json()['status'], 'no_layout_in_domain')
        self.assertIsNone(handoff)


if __name__ == '__main__':
    unittest.main()
