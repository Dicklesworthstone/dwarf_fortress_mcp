"""Full RoomPlan and allocation regressions on an actual 512-tile room project.

Ordinary repository imports; no replaced compiler, allocator, source selector,
protocol peer or validator. Geometry and inventory are synthetic planning input.
"""
from __future__ import annotations

from collections import deque
from copy import deepcopy
import itertools
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from furniture_allocation import Candidate, allocate
from furniture_plan import FurniturePlan, canonical
import room_provisioning as rooms


def request():
    return json.loads((ROOT / 'examples' / 'room_connection_budget_fallback.json').read_text())


def cells(region):
    return set(itertools.product(*(range(a, a + n) for a, n in zip(region['origin'], region['size']))))


def floor(plan):
    parts = [cells(part['region']) for part in plan['excavation_blueprint']['parts']]
    joined = set().union(*parts)
    if len(joined) != sum(map(len, parts)):
        raise AssertionError('excavation rectangles overlap')
    return joined


def neighbors(p):
    # Independent tile adjacency, not the production solver or router helpers.
    return tuple(tuple(n + (delta if axis == i else 0) for i, n in enumerate(p))
                 for axis in (0, 1) for delta in (-1, 1))


def reach(mask, root):
    seen, pending = {root}, deque([root])
    while pending:
        for point in neighbors(pending.popleft()):
            if point in mask and point not in seen:
                seen.add(point)
                pending.append(point)
    return seen


def bedroom_walls(body):
    walls = set()
    for area in body['areas']:
        if area['name'] != 'beds':
            continue
        for unit in area['units']:
            x, y, z = unit['origin']
            w, h, _ = unit['size']
            walls.update((xx, yy, z) for xx in range(x - 1, x + w + 1)
                         for yy in range(y - 1, y + h + 1)
                         if (xx in (x - 1, x + w) or yy in (y - 1, y + h))
                         and (xx, yy, z) != tuple(unit['doorway']))
    return walls


def three_component_oracle(floors, admitted):
    """Independent 0/1 shortest paths and three-way junction enumeration.

    For three free floor components, an optimal connecting tree has a common
    branching vertex. Sum the three node-cost distances there, removing two
    duplicate charges for a new junction. No subset DP or production trace.
    """
    pending, components = set(floors), []
    while pending:
        component = reach(pending, min(pending))
        components.append(component)
        pending.difference_update(component)
    if len(components) != 3:
        raise AssertionError('three-component fixture required')
    maps = []
    for component in components:
        distance = dict.fromkeys(component, 0)
        queue = deque(sorted(component))
        while queue:
            point = queue.popleft()
            for next_point in neighbors(point):
                if next_point not in admitted:
                    continue
                weight = int(next_point not in floors)
                value = distance[point] + weight
                if value < distance.get(next_point, 10**9):
                    distance[next_point] = value
                    (queue.append if weight else queue.appendleft)(next_point)
        maps.append(distance)
    common = set(maps[0]) & set(maps[1]) & set(maps[2])
    return min(sum(d[p] for d in maps) - 2 * int(p not in floors) for p in common)


class CompleteRoomConnectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.plan = rooms.RoomPlan.compile(request())

    def test_real_maximum_project_keeps_every_room_and_furnishing(self):
        body = self.plan.json()
        self.assertEqual(self.plan.digest, '7762688a0a039ac92193770db11d59bb0eaba7d729c281aa58660fd6fa0ce9a5')
        self.assertEqual(body['target_tiles'], 512)
        self.assertEqual(body['furniture_count'], 28)
        self.assertEqual(len(body['areas'][0]['units']), 8)
        self.assertEqual([a['name'] for a in body['areas']], ['beds', 'eat_a', 'eat_b'])
        self.assertEqual(len(body['excavation_blueprint']['parts']), 24)
        routing = body['project_connections']
        self.assertEqual(routing['policy'], 'dfmcp.room-project-minimum-connections/1')
        self.assertEqual(routing['new_corridor_tiles'], 13)
        self.assertLessEqual(routing['work_units'], 100000)
        self.assertTrue(routing['minimum_excavation_in_region_proven'])

    def test_large_project_minimum_matches_independent_junction_oracle(self):
        value = request()
        value['connect_areas'] = False
        unconnected = rooms.RoomPlan.compile(value).json()
        original = floor(unconnected)
        targets = {s.target for s in self.plan.request().slots}
        final = floor(self.plan.json())
        admitted = cells(self.plan.json()['capture_region']) - targets - bedroom_walls(unconnected)
        self.assertEqual(len(original), 499)
        self.assertEqual(three_component_oracle(original - targets, admitted), 13)
        self.assertEqual(len(final - original), 13)
        self.assertTrue(original <= final)

    def test_all_bedroom_walls_and_furniture_approaches_survive_lowering(self):
        body = self.plan.json()
        final = floor(body)
        targets = {s.target for s in self.plan.request().slots}
        self.assertFalse(final & bedroom_walls(body))
        walkable = final - targets
        connected = reach(walkable, tuple(body['areas'][0]['entry_point']))
        self.assertEqual(connected, walkable)
        for area in body['areas']:
            self.assertIn(tuple(area['entry_point']), connected)
            for approach in area['approaches']:
                self.assertIn(tuple(approach['approach']), connected)
        self.assertEqual(len(final), body['target_tiles'])
        self.assertTrue(final <= cells(body['capture_region']))

    def test_actual_allocator_consumes_complete_rescued_request(self):
        wanted = self.plan.request()
        candidates = tuple(Candidate(i, s.kind, s.target, (0, 1), -1)
                           for i, s in enumerate(wanted.slots))
        result = allocate(wanted, candidates)
        self.assertEqual(result['status'], 'allocated')
        actual = FurniturePlan.from_json(result['plan'])
        self.assertEqual(len(actual.steps), 28)
        self.assertEqual(result['total_distance'], 0)
        self.assertEqual({s.target for s in actual.steps}, {s.target for s in wanted.slots})
        self.assertEqual({s.name: s.after for s in actual.steps}, {s.name: s.after for s in wanted.slots})
        self.assertFalse(result['game_effect_performed'])

    def test_missing_item_cannot_turn_rescued_project_into_partial_plan(self):
        wanted = self.plan.request()
        candidates = tuple(Candidate(i, s.kind, s.target, (0, 1), -1)
                           for i, s in enumerate(wanted.slots[:-1]))
        result = allocate(wanted, candidates)
        self.assertEqual(result['status'], 'shortage')
        self.assertIsNone(result['plan'])
        self.assertEqual(result['assignments'], [])
        self.assertEqual(result['request_digest'], wanted.digest)

    def test_replay_and_request_order_preserve_exact_complete_artifact(self):
        self.assertEqual(rooms.RoomPlan.decode(self.plan.encode()), self.plan)
        value = request()
        before = deepcopy(value)
        self.assertEqual(rooms.RoomPlan.compile(value), self.plan)
        self.assertEqual(value, before)
        value['areas'].reverse()
        self.assertEqual(rooms.RoomPlan.compile(value), self.plan)

    def test_tampered_intent_or_derived_corridor_cannot_be_rehashed_into_proof(self):
        for mutation in ('digest', 'count', 'corridor', 'origin', 'minimum'):
            body = self.plan.json()
            if mutation == 'digest':
                body['project_connections']['connection_digest'] = '0' * 64
            elif mutation == 'count':
                body['target_tiles'] -= 1
            elif mutation == 'corridor':
                body['excavation_blueprint']['parts'][1]['region']['size'][1] -= 1
            elif mutation == 'origin':
                body['intent']['areas'][1]['origin'][0] += 1
            else:
                body['project_connections']['minimum_excavation_in_region_proven'] = False
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                rooms.RoomPlan.decode(canonical(body))

    def test_excluded_junction_or_passage_cannot_be_mined_to_fit_the_cap(self):
        for point in ([51, 28, 2], [53, 25, 2]):
            value = request()
            value['excluded_regions'] = [{'origin': point, 'size': [1, 1, 1]}]
            with self.subTest(point=point), self.assertRaisesRegex(ValueError, 'tile allowance'):
                rooms.RoomPlan.compile(value)

    def test_whole_compiler_cancellation_propagates_without_mutating_intent(self):
        value = request()
        before = deepcopy(value)
        checkpoints = []
        rooms.RoomPlan.compile(value, lambda: checkpoints.append(1))
        for stop in (1, 10, len(checkpoints) // 3, len(checkpoints) // 2,
                     len(checkpoints) - 1, len(checkpoints)):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('caller cancelled complete room compilation')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'caller cancelled'):
                rooms.RoomPlan.compile(value, guard)
            self.assertEqual(seen, stop)
            self.assertEqual(value, before)

    def test_legacy_complete_plan_digests_remain_unchanged(self):
        fixtures = [
            ({'kind': 'bedroom_cluster', 'rooms_count': 2, 'room_size': [3, 5]},
             '940c86dcdc5b6cf46734d83ed736ff98240b3cde75686d058edd20cfa2bb88e0'),
            ({'kind': 'dining_hall', 'table_count': 4, 'columns': 4},
             '1e3c4b8c48df802b5dc776633e2629c9982175abaee9065b76f2b47e62726a38')]
        for template, expected in fixtures:
            value = {'schema': rooms.INTENT_SCHEMA, 'world_folder': 'fallback-fixture', 'site': 7,
                     'connect_areas': True, 'areas': [{'name': 'legacy', 'origin': [80, 80, 2],
                         'rotation': 90, 'template': template}]}
            plan = rooms.RoomPlan.compile(value)
            self.assertEqual(plan.digest, expected)
            self.assertEqual(rooms.RoomPlan.decode(plan.encode()), plan)

    def test_concurrent_cross_level_selectors_survive_complete_fallback(self):
        value = request()
        for area in value['areas']:
            kinds = ('bed', 'chair', 'table') if area['name'] == 'beds' else ('chair', 'table')
            area['item_constraints'] = {kind: {'max_vertical_distance': 1} for kind in kinds}
        plan = rooms.RoomPlan.compile(value)
        self.assertEqual(floor(plan.json()), floor(self.plan.json()))
        self.assertTrue(plan.json()['project_connections']['minimum_excavation_in_region_proven'])
        self.assertTrue(all(slot.max_vertical_distance == 1 for slot in plan.request().slots))
        self.assertEqual(rooms.RoomPlan.decode(plan.encode()), plan)
        # Neither a valid room route nor same-level candidates may discard the
        # concurrent caller's observed-route-only furniture contract.
        with self.assertRaisesRegex(ValueError, 'observed route allocator'):
            allocate(plan.request(), ())

    def test_finite_geometry_proof_grants_no_native_or_room_completion_claim(self):
        body = self.plan.json()
        for key in ('terrain_observed', 'map_dimensions_observed', 'native_pathfinding_proven',
                    'existing_fort_access_proven', 'wall_preservation_observed', 'room_assignments_created',
                    'prepared_plan_created', 'room_completion_proven', 'mutation_authority_granted',
                    'construction_completion_proven'):
            self.assertIs(body[key], False, key)
        self.assertIs(body['project_connections']['global_minimum_excavation_proven'], False)


if __name__ == '__main__':
    unittest.main()
