"""Complete-project optimization checked against independent origin enumeration.

Real compiler/allocator modules only. Source masks are synthetic planning
constraints, not live terrain observations or mutation authority.
"""
from __future__ import annotations

from copy import deepcopy
import hashlib
import itertools
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import room_layout as layout
from room_layout_rank import Ranking
from room_provisioning import INTENT_SCHEMA, RoomPlan
from furniture_plan import canonical
from furniture_allocation import Candidate, allocate


def dining(name='a', count=1, **extra):
    return {'name': name, 'template': {'kind': 'dining_hall', 'table_count': count, 'columns': 4}, **extra}


def request(*areas, size=(8, 6, 1), **extra):
    return {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'ranking-fixture', 'site': 7,
            'region': {'origin': [10, 10, 2], 'size': list(size)},
            'areas': list(areas) or [dining('a'), dining('b')], **extra}


def optimized(value):
    return layout.solve(canonical({**value, 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}))


def entrance_example():
    return request(dining('hall', 4), size=(16, 10, 1), access_point=[25, 19, 2])


def joint_example():
    return request(dining('a', 4, origin=[10, 10, 2]), dining('b', 4),
                   size=(32, 6, 1), focus=[41, 15, 2])


def score(body, focus):
    # Independent tuple comparison, no production Ranking helper.
    distance = sum(sum(abs(p - f) for p, f in zip(area['entry_point'], focus))
                   for area in body['areas'])
    positions = tuple((a['origin'][1], a['origin'][0], a['origin'][2], a.get('rotation', 0))
                      for a in sorted(body['intent']['areas'], key=lambda a: a['name']))
    return body['target_tiles'], distance, positions


def points(region):
    return set(itertools.product(*(range(p, p + n) for p, n in zip(region['origin'], region['size']))))


def exhaustive(value):
    """Enumerate absolute origins/angles without solver bitsets or MRV helpers."""
    region = value['region']
    domain = points(region)
    choices = []
    for area in sorted(value['areas'], key=lambda a: a['name']):
        positions = [tuple(area['origin'])] if 'origin' in area else sorted(points(area.get('origin_region', region)))
        angles = area.get('rotations', [area.get('rotation', 0)])
        admitted = []
        for origin, angle in itertools.product(positions, angles):
            row = {k: v for k, v in area.items() if k not in ('origin_region', 'rotations', 'rotation')}
            row['origin'] = list(origin)
            if angle:
                row['rotation'] = angle
            try:
                plan = RoomPlan.compile({'schema': INTENT_SCHEMA, 'world_folder': value['world_folder'],
                                         'site': value['site'], 'areas': [row]}).json()
            except ValueError:
                continue
            cells = set().union(*(points(p['region']) for p in plan['excavation_blueprint']['parts']))
            if row['template']['kind'] == 'bedroom_cluster':
                for unit in plan['areas'][0]['units']:
                    x, y, z = unit['origin']; w, h, _ = unit['size']
                    cells.update((xx, yy, z) for xx in range(x - 1, x + w + 1)
                                 for yy in range(y - 1, y + h + 1))
            if cells <= domain:
                admitted.append(row)
        choices.append(admitted)
    best = None
    focus = value.get('focus', region['origin'])
    for rows in itertools.product(*choices):
        lowered = {'schema': INTENT_SCHEMA, 'world_folder': value['world_folder'], 'site': value['site'],
                   'areas': list(rows), 'connect_areas': True,
                   **{k: value[k] for k in ('excluded_items', 'excluded_regions', 'site_mask') if k in value}}
        if 'access_point' in value:
            lowered['access'] = {'point': value['access_point'], 'region': region}
        try:
            plan = RoomPlan.compile(lowered, _connection_fallback='connection_strategy' in value)
        except ValueError:
            continue
        candidate = score(plan.json(), focus)
        if best is None or candidate < best[0]:
            best = candidate, plan
    return best


def legacy_cases():
    for width, height, count, rotated, limit in itertools.product((4, 6, 8), (3, 5, 7), (1, 2), (False, True), (1, 3, 100)):
        areas = [dining('a', **({'rotations': [0, 90, 180, 270]} if rotated else {}))]
        if count == 2:
            areas.append(dining('b'))
        yield request(*areas, size=(width, height, 1), search={'max_nodes': limit})
    for access, fallback in itertools.product((False, True), (False, True)):
        row = entrance_example() if access else joint_example()
        if fallback:
            row['connection_strategy'] = layout.FALLBACK_STRATEGY
        yield row


def legacy_digest(solver):
    digest = hashlib.sha256()
    count = 0
    for row in legacy_cases():
        try:
            raw = solver(canonical(row)).encode()
        except ValueError as error:
            raw = canonical({'error': str(error)})
        digest.update(len(raw).to_bytes(8, 'big') + raw)
        count += 1
    return count, digest.hexdigest()


class RankedRoomLayoutTests(unittest.TestCase):
    def check(self, result, original):
        body = result.json()
        self.assertEqual(body['status'], 'layout_found')
        self.assertEqual(layout.Layout.decode(result.encode()), result)
        plan = result.room_plan()
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        self.assertEqual({a['name'] for a in plan.json()['areas']}, {a['name'] for a in original['areas']})
        self.assertFalse(body['partial_plan_returned'])
        self.assertFalse(body['global_optimum_proven'])
        self.assertEqual(body['optimization']['selected_plan_digest'], plan.digest)
        self.assertEqual(body['policy'], layout.OPTIMIZATION_POLICY)
        actual = score(plan.json(), body['request']['focus'])
        reported = body['optimization']['score']
        self.assertEqual((reported['target_tiles'], reported['total_entry_distance'],
                          tuple(tuple(p) for p in reported['placements_yxz_rotation'])), actual)
        return plan

    def test_entrance_connected_hall_avoids_ten_unnecessary_target_tiles(self):
        value = entrance_example()
        old = layout.solve(canonical(value)).json()
        result = optimized(value)
        plan = self.check(result, value)
        self.assertEqual(old['room_plan']['target_tiles'], 49)
        self.assertEqual(plan.json()['target_tiles'], 39)
        self.assertEqual(plan.json()['intent']['areas'][0]['origin'], [13, 17, 2])
        self.assertEqual(result.json()['optimization']['first_feasible_plan_digest'], old['room_plan']['plan_digest'])
        self.assertTrue(result.json()['optimization']['minimum_in_compiler_domain_proven'])
        self.assertFalse(result.json()['optimization']['actual_mining_work_observed'])

    def test_two_complete_halls_and_all_sixteen_furnishings_are_preserved(self):
        value = joint_example()
        old = layout.solve(canonical(value)).json()
        plan = self.check(optimized(value), value)
        self.assertEqual((old['room_plan']['target_tiles'], plan.json()['target_tiles']), (85, 78))
        self.assertEqual(plan.json()['furniture_count'], 16)
        wanted = plan.request()
        items = tuple(Candidate(i, s.kind, s.target, (0, 1), -1) for i, s in enumerate(wanted.slots))
        allocation = allocate(wanted, items)
        self.assertEqual(allocation['status'], 'allocated')
        self.assertEqual(len(allocation['plan']['steps']), 16)
        self.assertIsNone(allocate(wanted, items[:-1])['plan'])

    def test_independent_absolute_origin_and_rotation_oracle(self):
        for width, height in itertools.product((6, 7, 8), (4, 5, 6)):
            value = request(dining('a', rotations=[0, 90]), dining('b'), size=(width, height, 1))
            expected = exhaustive(value)
            actual = optimized(value).json()
            with self.subTest(width=width, height=height):
                if expected is None:
                    self.assertEqual(actual['status'], 'no_layout_in_domain')
                    self.assertIsNone(actual['room_plan'])
                else:
                    self.assertEqual(score(actual['room_plan'], value['region']['origin']), expected[0])
                    self.assertEqual(actual['room_plan'], expected[1].json())
                    self.assertTrue(actual['optimization']['minimum_in_compiler_domain_proven'])

    def test_every_small_obstacle_mask_matches_independent_oracle(self):
        blocks = [(10, 10, 2), (14, 10, 2), (10, 14, 2), (14, 14, 2), (12, 12, 2)]
        for bits in range(32):
            excluded = [{'origin': list(p), 'size': [1, 1, 1]} for i, p in enumerate(blocks) if bits & (1 << i)]
            value = request(dining(rotations=[0, 90, 180, 270]), size=(5, 5, 1), excluded_regions=excluded)
            expected = exhaustive(value)
            body = optimized(value).json()
            with self.subTest(mask=bits):
                self.assertEqual(body['room_plan'] is not None, expected is not None)
                if expected:
                    self.assertEqual(score(body['room_plan'], value['region']['origin']), expected[0])
                else:
                    self.assertTrue(body['search']['domain_exhausted'])
                    self.assertTrue(body['optimization']['search_complete'])
                    self.assertFalse(body['optimization']['minimum_in_compiler_domain_proven'])

    def test_node_limit_returns_only_complete_best_so_far_and_never_optimality(self):
        value = entrance_example()
        for limit in (1, 2, 7, 15, 32, 33):
            limited = {**value, 'search': {'max_nodes': limit}}
            out = optimized(limited)
            body = out.json()
            self.assertLessEqual(body['search']['nodes_visited'], limit)
            if limit == 1:
                self.assertEqual(body['status'], 'search_limit_reached')
                self.assertIsNone(body['room_plan'])
                self.assertFalse(body['all_areas_placed'])
            else:
                self.check(out, limited)
            self.assertEqual(body['optimization']['search_complete'], limit == 33)
            self.assertEqual(body['optimization']['minimum_in_compiler_domain_proven'], limit == 33)
            if limit < 33:
                self.assertEqual(body['optimization']['stop_reason'], 'node_limit_reached')

    def test_larger_node_budget_cannot_worsen_selected_score(self):
        previous = None
        for limit in range(2, 34):
            result = optimized({**entrance_example(), 'search': {'max_nodes': limit}}).json()
            current = score(result['room_plan'], result['request']['focus'])
            if previous is not None:
                self.assertLessEqual(current, previous)
            previous = current
        self.assertEqual(previous[0], 39)

    def test_one_work_allowance_includes_normalization_search_ranking_and_publication(self):
        value = request(dining(), size=(5, 3, 1))
        used = optimized(value).json()['search']['work_units']
        for maximum in (1, 30, used // 2, used - 1):
            with self.subTest(maximum=maximum), self.assertRaises(layout.WorkLimit):
                optimized({**value, 'search': {'max_work': maximum}})
        body = optimized({**value, 'search': {'max_work': used}}).json()
        self.assertEqual(body['search']['work_units'], used)
        self.assertTrue(body['optimization']['search_complete'])

    def test_cancellation_at_every_checkpoint_never_publishes_an_incumbent(self):
        value = request(dining(), size=(4, 3, 1), objective=layout.MINIMUM_TARGETS_OBJECTIVE)
        raw = canonical(value)
        calls = []
        layout.solve(raw, lambda: calls.append(1))
        for stop in range(1, len(calls) + 1):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('cancelled ranking')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'cancelled ranking'):
                layout.solve(raw, guard)
            self.assertEqual(seen, stop)

    def test_failed_caller_guard_after_first_incumbent_does_not_become_no_layout(self):
        original = Ranking.consider
        reached = []
        def consider(owner, plan):
            original(owner, plan)
            reached.append(plan.digest)
        def guard():
            if reached:
                raise ValueError('revoked after complete candidate')
        with patch.object(Ranking, 'consider', consider), self.assertRaisesRegex(ValueError, 'revoked after complete'):
            layout.solve(canonical({**entrance_example(), 'objective': layout.MINIMUM_TARGETS_OBJECTIVE}), guard)
        self.assertEqual(len(reached), 1)

    def test_lower_bound_prunes_without_changing_exhaustive_best(self):
        value = request()
        result = optimized(value).json()
        self.assertGreater(result['optimization']['branches_pruned_by_lower_bound'], 0)
        self.assertEqual(score(result['room_plan'], value['region']['origin']), exhaustive(value)[0])
        self.assertTrue(result['optimization']['minimum_in_compiler_domain_proven'])
        self.assertFalse(result['search']['all_layouts_ranked'])

    def test_allowed_angles_area_order_and_exclusion_order_are_canonical(self):
        value = request(dining('z', rotations=[270, 0, 90]), dining('a'), excluded_items=[8, 7])
        before = deepcopy(value)
        expected = optimized(value).encode()
        self.assertEqual(value, before)
        value['areas'].reverse()
        value['areas'][1]['rotations'].reverse()
        value['excluded_items'].reverse()
        self.assertEqual(optimized(value).encode(), expected)

    def test_fixed_origins_and_origin_regions_are_never_relaxed(self):
        value = request(dining('a', origin=[10, 10, 2]), dining('b', origin_region={
            'origin': [14, 12, 2], 'size': [1, 2, 1]}))
        body = self.check(optimized(value), value).json()
        area = {a['name']: a for a in body['intent']['areas']}
        self.assertEqual(area['a']['origin'], [10, 10, 2])
        self.assertIn(area['b']['origin'], ([14, 12, 2], [14, 13, 2]))
        self.assertEqual(score(body, value['region']['origin']), exhaustive(value)[0])

    def test_required_walls_item_constraints_and_maximum_selection_survive(self):
        areas = []
        for i in range(8):
            areas.append(dining(f'h{i}', 2, origin=[10 + (i % 2) * 8, 10 + (i // 2) * 4, 2],
                                item_constraints={'chair': {'material': [3, 4], 'subtype': 2,
                                                             'max_vertical_distance': 1, 'max_distance': 25}}))
        value = request(*areas, size=(16, 16, 1))
        plan = self.check(optimized(value), value)
        self.assertEqual(len(plan.request().slots), 32)
        for slot in plan.request().slots:
            if slot.kind == 'chair':
                self.assertEqual((slot.material, slot.subtype, slot.max_vertical_distance, slot.max_distance),
                                 ((3, 4), 2, 1, 25))
        bedroom = {'name': 'beds', 'origin': [12, 11, 2],
                   'template': {'kind': 'bedroom_cluster', 'rooms_count': 1, 'room_size': [3, 3]}}
        value = request(bedroom, dining('hall', origin_region={'origin': [10, 17, 2], 'size': [3, 1, 1]}), size=(12, 12, 1))
        plan = self.check(optimized(value), value)
        self.assertEqual(plan.json(), exhaustive(value)[1].json())

    def test_site_permissions_do_not_become_observed_mining_costs(self):
        value = request(dining(), size=(6, 4, 1))
        first = optimized(value).json()['room_plan']
        value['site_mask'] = {'region': value['region'], 'cells': 'F' * 24}
        second = optimized(value).json()['room_plan']
        self.assertEqual(first['target_tiles'], second['target_tiles'])
        self.assertFalse(second['terrain_observed'])
        value['site_mask']['cells'] = 'W' * 24
        out = optimized(value).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertIsNone(out['optimization']['score'])

    def test_objective_is_a_closed_exact_request_field(self):
        for objective in (None, True, False, 1, 1.0, [], {}, '', 'minimum-target-tiles', 'fastest'):
            with self.subTest(objective=objective), self.assertRaisesRegex(ValueError, 'objective'):
                layout.solve(canonical(request(objective=objective)))
        value = request()
        value['search'] = {'objective': layout.MINIMUM_TARGETS_OBJECTIVE}
        with self.assertRaises(ValueError):
            layout.solve(canonical(value))

    def test_altered_score_trace_objective_and_optimality_reject_saved_artifacts(self):
        result = optimized({**entrance_example(), 'search': {'max_nodes': 3}})
        mutations = []
        for field, val in (('search_complete', True), ('minimum_in_compiler_domain_proven', True),
                           ('first_feasible_plan_digest', '0' * 64)):
            value = result.json(); value['optimization'][field] = val; mutations.append(value)
        value = result.json(); value['optimization']['score']['target_tiles'] -= 1; mutations.append(value)
        value = result.json(); value['request'].pop('objective'); mutations.append(value)
        value = result.json(); value['search']['trace_digest'] = '0' * 64; mutations.append(value)
        value = result.json(); value['room_plan']['furniture_count'] -= 1; mutations.append(value)
        for value in mutations:
            with self.assertRaises(ValueError):
                layout.Layout.decode(canonical(value))

    def test_request_and_result_dicts_do_not_alias_retained_candidates(self):
        value = entrance_example()
        old = deepcopy(value)
        result = optimized(value)
        raw = result.encode()
        result.json()['room_plan']['areas'].clear()
        self.assertEqual(value, old)
        self.assertEqual(result.encode(), raw)
        self.assertEqual(layout.Layout.decode(raw), result)

    def test_original_fallback_remains_an_independent_sealed_choice(self):
        intent = json.loads((ROOT / 'examples' / 'room_connection_budget_fallback.json').read_bytes())
        value = {'schema': layout.REQUEST_SCHEMA, 'world_folder': intent['world_folder'], 'site': intent['site'],
                 'areas': intent['areas'], 'region': {'origin': [18, 15, 2], 'size': [39, 24, 1]}}
        old = optimized(value).json()
        self.assertEqual(old['status'], 'no_layout_in_domain')
        value['connection_strategy'] = layout.FALLBACK_STRATEGY
        plan = self.check(optimized(value), value)
        self.assertEqual(plan.json()['target_tiles'], 512)
        self.assertTrue(plan.json()['project_connections']['minimum_excavation_in_region_proven'])
        with patch('room_connection_solver.MAX_COMPONENTS', 1), self.assertRaises(layout.WorkLimit):
            optimized(value)

    def test_legacy_corpus_retains_bytes_choices_traces_and_work_counts(self):
        self.assertEqual(legacy_digest(layout.solve), LEGACY_GOLDEN)


# Populated from the exact pre-change room_layout.py blob, not the new solver.
LEGACY_GOLDEN = (112, 'e7dabea28f8a817ce72e5fcc4542cf141dd5225c8bb32d0766b1ff5d2c85b0cc')


if __name__ == '__main__':
    unittest.main()
