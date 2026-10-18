"""Replay-safe automatic layout integration through the complete real compiler."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import room_connection_solver as solver
import room_connections as connections
import room_layout as layout
from room_provisioning import RoomPlan
from furniture_plan import canonical


def request(*, alternatives=False, fallback=False):
    value = json.loads((ROOT / 'examples/room_connection_budget_fallback.json').read_bytes())
    value.pop('connect_areas')
    value.update(schema=layout.REQUEST_SCHEMA, region={'origin': [18, 15, 2], 'size': [39, 24, 1]})
    if alternatives:
        value['areas'][1].pop('origin')
        value['areas'][1]['origin_region'] = {'origin': [55, 15, 2], 'size': [1, 2, 1]}
    if fallback:
        value['connection_strategy'] = layout.FALLBACK_STRATEGY
    return value


def solve(value, guard=layout._idle):
    return layout.solve(canonical(value), guard)


class LayoutFallbackTests(unittest.TestCase):
    def test_legacy_refusal_replays_unchanged_instead_of_becoming_a_new_plan(self):
        result = solve(request())
        self.assertEqual(result.digest, 'ae5d33e6b8a6bd44e8296b21b10370adce2f04d91667fd41f3e885daa1cd8052')
        self.assertEqual(result.json()['status'], 'no_layout_in_domain')
        self.assertEqual(result.json()['search']['work_units'], 23998)
        self.assertEqual(layout.Layout.decode(result.encode()), result)

    def test_legacy_success_replays_original_later_candidate_and_work_counts(self):
        result = solve(request(alternatives=True))
        self.assertEqual(result.digest, 'a13e618c17721b8910aef1f75632bb5a3235ed9d332519800ac44c18ba079f4f')
        self.assertEqual(result.json()['search']['compiler_refusals'], 1)
        self.assertEqual(result.json()['search']['work_units'], 42228)
        self.assertEqual(result.room_plan().json()['intent']['areas'][1]['origin'], [55, 16, 2])
        self.assertEqual(layout.Layout.decode(result.encode()), result)

    def test_explicit_strategy_rescues_complete_project_and_replays(self):
        value = request(fallback=True)
        before = deepcopy(value)
        result = solve(value)
        body = result.json()
        self.assertEqual(value, before)
        self.assertEqual(body['status'], 'layout_found')
        self.assertEqual(body['policy'], layout.FALLBACK_POLICY)
        self.assertEqual(body['request']['connection_strategy'], layout.FALLBACK_STRATEGY)
        plan = result.room_plan()
        self.assertEqual((plan.json()['target_tiles'], plan.json()['furniture_count']), (512, 28))
        self.assertEqual(plan.json()['project_connections']['new_corridor_tiles'], 13)
        self.assertTrue(plan.json()['project_connections']['minimum_excavation_in_region_proven'])
        self.assertEqual(RoomPlan.decode(plan.encode()), plan)
        self.assertEqual(layout.Layout.decode(result.encode()), result)

    def test_explicit_strategy_can_choose_earlier_candidate_without_rewriting_old_result(self):
        old = solve(request(alternatives=True))
        old_raw = old.encode()
        new = solve(request(alternatives=True, fallback=True))
        self.assertEqual(new.room_plan().json()['intent']['areas'][1]['origin'], [55, 15, 2])
        self.assertEqual(new.json()['search']['compiler_refusals'], 0)
        self.assertNotEqual(new.digest, old.digest)
        self.assertEqual(layout.Layout.decode(old_raw).encode(), old_raw)

    def test_strategy_is_a_closed_exact_value(self):
        for invalid in (True, False, None, 0, 1, [], {}, 'greedy', 'minimum',
                        layout.FALLBACK_STRATEGY + ' '):
            value = request()
            value['connection_strategy'] = invalid
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, 'strategy'):
                solve(value)

    def test_removed_or_changed_strategy_and_policy_cannot_forge_replay(self):
        original = solve(request(fallback=True))
        for mutation in ('strategy', 'policy', 'plan', 'trace'):
            body = original.json()
            if mutation == 'strategy':
                body['request'].pop('connection_strategy')
            elif mutation == 'policy':
                body['policy'] = layout.ORIENTED_POLICY
            elif mutation == 'plan':
                body['room_plan']['target_tiles'] -= 1
            else:
                body['search']['work_units'] -= 1
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                layout.Layout.decode(canonical(body))

    def test_strategy_is_sealed_even_when_greedy_already_fits(self):
        value = {'schema': layout.REQUEST_SCHEMA, 'world_folder': 'fixture', 'site': 7,
                 'region': {'origin': [10, 10, 2], 'size': [4, 3, 1]},
                 'areas': [{'name': 'hall', 'template': {'kind': 'dining_hall', 'table_count': 1}}]}
        old = solve(value)
        value['connection_strategy'] = layout.FALLBACK_STRATEGY
        new = solve(value)
        self.assertEqual(new.room_plan(), old.room_plan())
        self.assertNotEqual(new.digest, old.digest)
        self.assertEqual(layout.Layout.decode(new.encode()), new)

    def test_one_shared_work_budget_covers_greedy_and_fallback(self):
        value = request(fallback=True)
        used = solve(value).json()['search']['work_units']
        for maximum in (1, used // 2, used - 1):
            with self.subTest(maximum=maximum), self.assertRaises(layout.WorkLimit):
                solve({**value, 'search': {'max_work': maximum}})
        exact = solve({**value, 'search': {'max_work': used}})
        self.assertEqual(exact.json()['status'], 'layout_found')
        self.assertEqual(exact.json()['search']['work_units'], used)
        self.assertEqual(layout.Layout.decode(exact.encode()), exact)

    def test_component_bound_is_inconclusive_at_the_real_search_boundary(self):
        # Inject a smaller resource capacity, not a different compiler or solver.
        with patch.object(solver, 'MAX_COMPONENTS', 2):
            with self.assertRaises(layout.WorkLimit):
                solve(request(fallback=True))
        self.assertEqual(solve(request(fallback=True)).json()['status'], 'layout_found')

    def test_node_exhaustion_never_exports_an_incomplete_room_plan(self):
        result = solve({**request(fallback=True), 'search': {'max_nodes': 3}})
        body = result.json()
        self.assertEqual(body['status'], 'search_limit_reached')
        self.assertIsNone(body['room_plan'])
        self.assertFalse(body['search']['domain_exhausted'])
        with self.assertRaises(ValueError):
            result.room_plan()
        self.assertEqual(layout.Layout.decode(result.encode()), result)

    def test_caller_cancellation_in_fallback_is_not_a_candidate_refusal(self):
        value = request(fallback=True)
        calls = []
        solve(value, lambda: calls.append(1))
        for stop in (1, 20000, len(calls) // 2, len(calls) - 1, len(calls)):
            seen = 0
            def guard():
                nonlocal seen
                seen += 1
                if seen == stop:
                    raise ValueError('caller cancelled entire layout')
            with self.subTest(stop=stop), self.assertRaisesRegex(ValueError, 'caller cancelled'):
                solve(value, guard)
            self.assertEqual(seen, stop)

    def test_no_global_strategy_state_leaks_between_calls(self):
        first = solve(request(alternatives=True)).encode()
        solve(request(fallback=True))
        self.assertEqual(solve(request(alternatives=True)).encode(), first)
        with patch.object(solver, 'MAX_COMPONENTS', 2), self.assertRaises(layout.WorkLimit):
            solve(request(fallback=True))
        self.assertEqual(solve(request(alternatives=True)).encode(), first)

    def test_constraint_cannot_be_added_as_an_unsealed_room_intent_field(self):
        original = json.loads((ROOT / 'examples/room_connection_budget_fallback.json').read_bytes())
        original['connection_strategy'] = layout.FALLBACK_STRATEGY
        with self.assertRaisesRegex(ValueError, 'fields'):
            RoomPlan.compile(original)
        original.pop('connection_strategy')
        for invalid in (None, 1, 'yes'):
            with self.assertRaisesRegex(ValueError, 'fallback selection'):
                RoomPlan.compile(original, _connection_fallback=invalid)

    def test_request_order_and_process_hash_seed_are_deterministic(self):
        value = request(alternatives=True, fallback=True)
        expected = solve(value).encode()
        value['areas'].reverse()
        self.assertEqual(solve(value).encode(), expected)
        code = 'import sys,room_layout;sys.stdout.buffer.write(room_layout.solve(sys.stdin.buffer.read()).encode())'
        for seed in ('1', '27', 'random'):
            done = subprocess.run([sys.executable, '-c', code], input=canonical(value), capture_output=True,
                env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1', 'PYTHONPATH': str(ROOT / 'scripts'),
                     'PYTHONHASHSEED': seed}, check=True, timeout=15)
            self.assertEqual(done.stdout, expected)


if __name__ == '__main__':
    unittest.main()
