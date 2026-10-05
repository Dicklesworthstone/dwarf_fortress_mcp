"""Complete restart handoffs for df-dfhack-bridge-plane-c-pic.

These are pure Python development tests, not native placement qualification.
Run: python3 -m unittest discover -s tests -p 'test_furniture_plan_handoff.py' -v
"""
from __future__ import annotations

import itertools
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))

from furniture_plan import MAX_BYTES, FurniturePlan, Step, canonical, progress


def example() -> FurniturePlan:
    # Lexical storage order intentionally differs from dependency order.
    return FurniturePlan((
        Step('a-chair', 'chair', 42, (3, 2, 0), ('z-bed',)),
        Step('m-table', 'table', 43, (4, 2, 0), ('a-chair',)),
        Step('z-bed', 'bed', 41, (2, 2, 0)),
    ))


class FurniturePlanHandoffTests(unittest.TestCase):
    def assert_complete_handoff(self, plan: FurniturePlan, result: dict) -> None:
        self.assertEqual(result['plan'], plan.json())
        restored = FurniturePlan.decode(canonical(result['plan']))
        self.assertEqual(restored, plan)
        self.assertEqual(result['plan_digest'], restored.digest)
        self.assertEqual([row['name'] for row in result['steps']],
                         [step.name for step in restored.ordered])
        self.assertFalse(result['atomic'])
        self.assertFalse(result['construction_completion_proven'])
        self.assertFalse(result['retry_permitted'])

    def test_new_batch_discloses_all_unstarted_selections(self) -> None:
        plan = example()
        result = progress(plan, {})
        self.assert_complete_handoff(plan, result)
        self.assertEqual(result['next_step'], 'z-bed')
        self.assertEqual(result['placed'], 0)
        self.assertEqual({step['item'] for step in result['plan']['steps']}, {41, 42, 43})
        self.assertEqual({tuple(step['target']) for step in result['plan']['steps']},
                         {(2, 2, 0), (3, 2, 0), (4, 2, 0)})

    def test_each_placed_prefix_retains_the_whole_original_plan(self) -> None:
        plan = example()
        for count in range(len(plan.steps) + 1):
            with self.subTest(count=count):
                result = progress(plan, {s.name: 'placed' for s in plan.ordered[:count]})
                self.assert_complete_handoff(plan, result)
                self.assertEqual(result['placed'], count)
                self.assertEqual(result['status'], 'all_placed' if count == 3 else 'ready')
                self.assertEqual(result['next_step'], None if count == 3 else plan.ordered[count].name)

    def test_pending_and_halted_handoffs_preserve_future_work(self) -> None:
        plan = example()
        for phase in ('unknown', 'indeterminate', 'refused', 'cancelled'):
            with self.subTest(phase=phase):
                result = progress(plan, {'z-bed': 'placed', 'a-chair': phase})
                self.assert_complete_handoff(plan, result)
                self.assertIsNone(result['next_step'])
                self.assertEqual(result['steps'][-1]['phase'], 'not_started')
                expected = 'pending_recovery' if phase in ('unknown', 'indeterminate') else 'halted_' + phase
                self.assertEqual(result['status'], expected)
                self.assertEqual(result['pending_step'], 'a-chair' if expected == 'pending_recovery' else None)

    def test_dictionary_and_plan_input_order_do_not_change_handoff_bytes(self) -> None:
        plan = example()
        expected = canonical(progress(plan, {'z-bed': 'placed', 'a-chair': 'indeterminate'}))
        for steps in itertools.permutations(plan.steps):
            for records in ((('z-bed', 'placed'), ('a-chair', 'indeterminate')),
                            (('a-chair', 'indeterminate'), ('z-bed', 'placed'))):
                self.assertEqual(canonical(progress(FurniturePlan(steps), dict(records))), expected)

    def test_batch_shaping_may_remove_duplicate_dependencies_without_losing_them(self) -> None:
        plan = example()
        result = progress(plan, {'z-bed': 'placed'})
        # Batch.audit removes these duplicated presentation fields. The original
        # plan must still be enough for a fresh caller to reconstruct the DAG.
        for row in result['steps']:
            row.pop('dependencies')
        self.assert_complete_handoff(plan, result)
        self.assertEqual(result['plan']['steps'][0]['after'], ['z-bed'])

    def test_returned_json_does_not_mutate_retained_plan(self) -> None:
        plan = example()
        expected = canonical(progress(plan, {}))
        result = progress(plan, {})
        result['plan']['steps'][0]['item'] = 999
        result['plan']['steps'][0]['target'][0] = 99
        result['plan']['steps'][0]['after'].clear()
        result['plan']['steps'].clear()
        self.assertEqual(canonical(progress(plan, {})), expected)

    def test_null_record_cannot_be_silently_reclassified_as_not_started(self) -> None:
        for name in ('z-bed', 'a-chair', 'm-table'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                progress(example(), {name: None})

    def test_unknown_record_values_are_rejected(self) -> None:
        for value in ('', 'prepared', 'verified', 'not_started', 0, False, [], {}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                progress(example(), {'z-bed': value})

    def test_nonprefix_records_remain_rejected(self) -> None:
        for records in ({'a-chair': 'placed'}, {'z-bed': 'unknown', 'a-chair': 'placed'},
                        {'z-bed': 'refused', 'a-chair': 'unknown'}, {'foreign': 'placed'}):
            with self.subTest(records=records), self.assertRaises(ValueError):
                progress(example(), records)

    def test_near_limit_32_step_dag_retains_every_target_and_dependency(self) -> None:
        accepted = []
        for width in range(3, 49):
            names = [f'{index:02d}' + 'x' * (width - 2) for index in range(32)]
            try:
                plan = FurniturePlan(tuple(Step(name, 'chair', 2147483646 - index,
                    (32700 + index, 32766, 32767), tuple(names[:index]))
                    for index, name in enumerate(names)))
            except ValueError:
                continue
            accepted.append(plan)
        self.assertTrue(accepted)
        plan = accepted[-1]
        self.assertGreater(len(canonical(plan.json())), MAX_BYTES - 1024)
        for count in range(33):
            result = progress(plan, {s.name: 'placed' for s in plan.ordered[:count]})
            self.assert_complete_handoff(plan, result)
            self.assertLess(len(canonical(result)), 49152)
            for row in result['steps']:
                row.pop('dependencies')
            # Preserve headroom for the batch envelope and the existing 32 KiB
            # native-outcome reservation; never truncate the original plan.
            self.assertLess(len(canonical(result)), 24576)


if __name__ == '__main__':
    unittest.main()
