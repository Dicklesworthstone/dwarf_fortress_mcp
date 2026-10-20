"""Complete bounded-cost layouts through the real compiler, allocator and CLI."""
from __future__ import annotations

from collections import deque
from copy import deepcopy
import heapq
import itertools
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
from room_provisioning import INTENT_SCHEMA, RoomPlan
import room_layout as layout
import room_cost_connections as routing
import room_bounded_cost_paths as paths
import layout_rooms as cli


def large_request():
    return json.loads((ROOT / 'examples/room_bounded_cost_layout.json').read_text())


def small_request(movable=False):
    value = {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'bounded-layout', 'site': 7,
             'region': {'origin': [10, 9, 2], 'size': [10, 5, 1]},
             'areas': [{'name': name, 'origin': [x, 10, 2],
                        'template': {'kind': 'dining_hall', 'table_count': 1}}
                       for name, x in (('a', 10), ('b', 16))],
             'objective': layout.COST_OBJECTIVE, 'excavation_costs': '1' * 50,
             'connection_strategy': layout.BOUNDED_COST_CONNECTION_STRATEGY}
    if movable:
        area = value['areas'][1]
        area.pop('origin')
        area['origin_region'] = {'origin': [16, 10, 2], 'size': [1, 2, 1]}
    return value


def cells(region):
    return set(itertools.product(*(range(a, a + n) for a, n in zip(region['origin'], region['size']))))


def targets(body):
    parts = [cells(p['region']) for p in body['excavation_blueprint']['parts']]
    union = set().union(*parts)
    if len(union) != sum(map(len, parts)):
        raise AssertionError('overlapping selected rectangles')
    return union


def adjacent(p):
    x, y, z = p
    return ((x-1, y, z), (x+1, y, z), (x, y-1, z), (x, y+1, z))


def flood(mask, start):
    visited, queue = {start}, deque([start])
    while queue:
        for point in adjacent(queue.popleft()):
            if point in mask and point not in visited:
                visited.add(point)
                queue.append(point)
    return visited


def original_plan(value):
    return RoomPlan.compile({'schema': INTENT_SCHEMA, 'world_folder': value['world_folder'],
                             'site': value['site'], 'areas': value['areas']}).json()


def protected(body):
    walls = set()
    kinds = {a['name']: a['template']['kind'] for a in body['intent']['areas']}
    for area in body['areas']:
        if kinds[area['name']] != 'bedroom_cluster':
            continue
        for unit in area['units']:
            x, y, z = unit['origin']; w, h, _ = unit['size']
            walls.update((xx, yy, z) for xx in range(x-1, x+w+1) for yy in range(y-1, y+h+1)
                         if (xx in (x-1, x+w) or yy in (y-1, y+h))
                         and (xx, yy, z) != tuple(unit['doorway']))
    return walls | {tuple(s['target']) for s in body['furniture_request']['slots']}


def independent_two_component_cost(value, body):
    """Full layered graph, not the production nondominated-label algorithm."""
    floors = targets(body)
    blocked = protected(body)
    walkable = floors - blocked
    root = tuple(body['areas'][0]['entry_point'])
    reached = flood(walkable, root)
    other = walkable - reached
    if not other or flood(other, min(other)) != other:
        raise AssertionError('oracle requires exactly two original components')
    admitted = cells(value['region']) - blocked
    low = value['region']['origin']; width = value['region']['size'][0]
    allowance = 512 - len(floors)
    distance = {(p, 0): 0 for p in reached}
    queue = [(0, 0, p) for p in sorted(reached)]
    heapq.heapify(queue)
    while queue:
        cost, added, point = heapq.heappop(queue)
        if distance[(point, added)] != cost:
            continue
        if point in other:
            return cost, added
        for nxt in adjacent(point):
            if nxt not in admitted:
                continue
            new = added + int(nxt not in walkable)
            if new > allowance:
                continue
            toll = int(value['excavation_costs'][(nxt[1]-low[1])*width+nxt[0]-low[0]])
            score = cost + (toll if nxt not in walkable else 0)
            key = nxt, new
            if score < distance.get(key, 10**9):
                distance[key] = score
                heapq.heappush(queue, (score, new, nxt))
    return None


class BoundedLayoutTests(unittest.TestCase):
    def test_full_project_improves_five_wall_corridor_to_one_at_512_tiles(self):
        value = large_request()
        before = deepcopy(value)
        old = layout.solve(canonical({**value, 'connection_strategy': layout.COST_CONNECTION_STRATEGY})).json()
        result = layout.solve(canonical(value))
        body = result.json(); plan = result.room_plan(); selected = plan.json()
        self.assertEqual(value, before)
        self.assertEqual(old['room_plan']['project_connections']['weighted_candidate_status'], 'tile_limit')
        self.assertEqual(old['optimization']['score']['excavation_cost'], 5)
        self.assertEqual(body['optimization']['score']['excavation_cost'], 1)
        self.assertEqual(selected['target_tiles'], 512)
        self.assertLessEqual(len(selected['excavation_blueprint']['parts']), 32)
        self.assertEqual(selected['furniture_count'], 28)
        self.assertEqual(selected['areas'], original_plan(value)['areas'])
        self.assertEqual(plan.request(), RoomPlan.decode(canonical(old['room_plan'])).request())
        self.assertEqual(layout.Layout.decode(result.encode()), result)
        self.assertEqual(body['policy'], layout.BOUNDED_COST_CONNECTION_POLICY)
        self.assertEqual(selected['intent']['corridor_costs']['strategy'], routing.BOUNDED_STRATEGY)
        self.assertEqual(selected['project_connections']['policy'], routing.BOUNDED_POLICY)
        self.assertEqual(selected['project_connections']['selected_strategy'], 'bounded_cost_first')

    def test_independent_layered_graph_confirms_one_wall_thirteen_tile_route(self):
        value = large_request(); original = original_plan(value)
        self.assertEqual(len(targets(original)), 499)
        self.assertEqual(independent_two_component_cost(value, original), (1, 13))
        selected = layout.solve(canonical(value)).room_plan().json()
        added = targets(selected) - targets(original)
        self.assertEqual(len(added), 13)
        self.assertFalse(added & protected(original))
        walkable = targets(selected) - protected(original)
        self.assertEqual(flood(walkable, tuple(selected['areas'][0]['entry_point'])), walkable)

    def test_all_original_furniture_allocates_or_reports_whole_shortage(self):
        wanted = layout.solve(canonical(large_request())).room_plan().request()
        candidates = tuple(Candidate(i, slot.kind, slot.target, (0, 1), -1)
                           for i, slot in enumerate(wanted.slots))
        found = allocate(wanted, candidates)
        self.assertEqual(found['status'], 'allocated')
        self.assertEqual(len(found['plan']['steps']), 28)
        self.assertEqual(found['total_distance'], 0)
        short = allocate(wanted, candidates[:-1])
        self.assertEqual(short['status'], 'shortage')
        self.assertIsNone(short['plan'])
        self.assertEqual(short['assignments'], [])

    def test_all_eight_areas_and_32_furnishings_are_retained(self):
        value = small_request()
        value['region'] = {'origin': [10, 10, 2], 'size': [32, 7, 1]}
        value['excavation_costs'] = '1' * 224
        value['areas'] = [{'name': str(i), 'origin': [10+(i%4)*8, 10+(i//4)*4, 2],
                           'template': {'kind': 'dining_hall', 'table_count': 2}} for i in range(8)]
        result = layout.solve(canonical(value))
        self.assertEqual(result.room_plan().json()['furniture_count'], 32)
        self.assertEqual(len(result.room_plan().json()['areas']), 8)
        self.assertEqual(layout.Layout.decode(result.encode()), result)

    def test_residual_capacity_accepts_exact_one_wall_proposal(self):
        value = {**large_request(), 'residual_partition': 'bounded-original-dig/1'}
        body = layout.solve(canonical(value)).json()
        proof = body['residual_partition']['selected']
        self.assertTrue(proof['within_profile_limits'])
        self.assertEqual((proof['remaining_tiles'], proof['blueprint_parts'], proof['normal_mining_rectangles']), (1, 1, 1))

    def test_exact_strategy_requires_the_cost_objective_and_full_cost_map(self):
        for update in ({'objective': layout.MINIMUM_TARGETS_OBJECTIVE}, {'objective': None},
                       {'excavation_costs': ''}, {'excavation_costs': '0'*49},
                       {'connection_strategy': True}, {'connection_strategy': []},
                       {'connection_strategy': 'bounded-cost-with-original-candidate/2'}):
            with self.subTest(update=update), self.assertRaises(ValueError):
                layout.solve(canonical({**small_request(), **update}))
        value = small_request(); value.pop('objective'); value.pop('excavation_costs')
        with self.assertRaises(ValueError):
            layout.solve(canonical(value))

    def test_fixed_origins_and_allowed_rotations_stay_binding(self):
        value = small_request()
        for area in value['areas']:
            area['rotations'] = [0]
        selected = layout.solve(canonical(value)).room_plan().json()['intent']['areas']
        self.assertEqual([a['origin'] for a in selected], [a['origin'] for a in value['areas']])
        self.assertTrue(all(a.get('rotation', 0) == 0 for a in selected))
        value['areas'][1]['origin'] = value['areas'][0]['origin']
        out = layout.solve(canonical(value)).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertIsNone(out['room_plan'])

    def test_cannot_cross_an_excluded_wall_to_connect_rooms(self):
        value = small_request()
        value['excluded_regions'] = [{'origin': [14, 9, 2], 'size': [1, 5, 1]}]
        out = layout.solve(canonical(value)).json()
        self.assertEqual(out['status'], 'no_layout_in_domain')
        self.assertIsNone(out['room_plan'])

    def test_node_limited_incumbent_is_complete_and_not_proven_optimal(self):
        value = small_request(movable=True)
        value['search'] = {'max_nodes': 3}
        result = layout.solve(canonical(value)); body = result.json()
        self.assertEqual(body['status'], 'layout_found')
        self.assertFalse(body['optimization']['minimum_in_compiler_domain_proven'])
        self.assertEqual(result.room_plan().json()['furniture_count'], 4)
        self.assertEqual(layout.Layout.decode(result.encode()), result)

    def test_label_exhaustion_does_not_return_the_already_computed_baseline(self):
        with patch.object(paths, 'MAX_LABELS', 1), self.assertRaises(layout.WorkLimit):
            layout.solve(canonical(small_request()))

    def test_work_allowance_covers_the_whole_original_search(self):
        value = small_request()
        used = layout.solve(canonical(value)).json()['search']['work_units']
        with self.assertRaises(layout.WorkLimit):
            layout.solve(canonical({**value, 'search': {'max_work': used-1}}))
        exact = layout.solve(canonical({**value, 'search': {'max_work': used}}))
        self.assertEqual(exact.json()['search']['work_units'], used)

    def test_caller_cancellation_is_not_misreported_as_infeasibility(self):
        raw = canonical(small_request()); count = 0
        def count_guard():
            nonlocal count
            count += 1
        layout.solve(raw, count_guard)
        for stop in sorted(set([1, count] + list(range(1, count+1, 137)))):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('external cancellation')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'external cancellation'):
                layout.solve(raw, guard)
            self.assertEqual(seen, stop)

    def test_rehashed_strategy_or_cost_tampering_is_not_accepted(self):
        result = layout.solve(canonical(large_request()))
        for name in ('request', 'room_plan', 'costs'):
            body = result.json(); body.pop('layout_digest')
            if name == 'request':
                body['request']['connection_strategy'] = layout.COST_CONNECTION_STRATEGY
            elif name == 'room_plan':
                body['room_plan']['intent']['corridor_costs'].pop('strategy')
            else:
                body['request']['excavation_costs'] = '0'*len(body['request']['excavation_costs'])
            with self.subTest(name=name), self.assertRaises(ValueError):
                layout.Layout.decode(layout.Layout(canonical(body)).encode())

    def test_real_offline_cli_exports_replay_and_strict_node_refusal(self):
        value = small_request(); result = layout.solve(canonical(value))
        with tempfile.TemporaryDirectory() as directory:
            request, saved = Path(directory)/'request.json', Path(directory)/'layout.json'
            request.write_bytes(canonical(value)); saved.write_bytes(result.encode())
            for option, source in (('--request-file', request), ('--layout-file', saved)):
                for kind in cli.EXPORTS:
                    run = subprocess.run([sys.executable, str(ROOT/'scripts/layout_rooms.py'), option,
                        str(source), '--emit', kind, '--require-optimal-layout'], capture_output=True, timeout=15)
                    self.assertEqual(run.returncode, 0, run.stdout[:300])
                    self.assertEqual(run.stdout, cli.export(result, kind))
            value = small_request(movable=True); value['search'] = {'max_nodes': 3}
            request.write_bytes(canonical(value))
            run = subprocess.run([sys.executable, str(ROOT/'scripts/layout_rooms.py'), '--request-file',
                str(request), '--emit', 'room-plan', '--require-optimal-layout'], capture_output=True, timeout=15)
            self.assertEqual(run.returncode, 2)
            self.assertEqual(json.loads(run.stdout)['error']['code'], 'ROOM_LAYOUT_OPTIMUM_UNPROVEN')

    def test_hash_seeds_preserve_complete_bounded_search_bytes(self):
        raw = canonical(small_request()); expected = layout.solve(raw).encode()
        code = 'import sys,room_layout;sys.stdout.buffer.write(room_layout.solve(sys.stdin.buffer.read()).encode())'
        for seed in ('1', '93', 'random'):
            run = subprocess.run([sys.executable, '-c', code], input=raw, capture_output=True, timeout=15,
                env={**os.environ, 'PYTHONPATH': str(ROOT/'scripts'), 'PYTHONHASHSEED': seed}, check=True)
            self.assertEqual(run.stdout, expected)


if __name__ == '__main__':
    unittest.main()
