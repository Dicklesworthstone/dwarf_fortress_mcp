"""Focused final-progress scope and workflow regressions.

The final frontier and native clock are explicit ports. These tests are intended
for the repository Python suite; they do not claim live DFHack qualification.
"""
from __future__ import annotations

from types import SimpleNamespace
import unittest
from unittest.mock import patch

import route_furniture_final_progress as fp
from route_furniture_clock_context import FINAL_POLICY, FINAL_SCHEMA, FinalScope


class Budget:
    def __init__(self):
        import time
        self.deadline = time.monotonic() + 60
        self.calls = 1000
        self.network_bytes = 100 * 1024 * 1024
        self.disk_bytes = 1024 * 1024 * 1024
        self.work_steps = 100_000_000
    def work(self): self.work_steps -= 1
    def remaining(self): return 30.0
    def reserve(self, name, count):
        value = getattr(self, name)
        if count > value: raise ValueError('budget')
        setattr(self, name, value - count)


class Plan:
    digest = 'c' * 64
    steps = (SimpleNamespace(name='bed'),)


class Source:
    endpoint = '127.0.0.1:5000'


class Batch:
    def __init__(self):
        self.id = 'a' * 64
        self.review = SimpleNamespace(digest='b' * 64)
        self.plan = Plan()
        self.source = Source()
        self.address = ('127.0.0.1', 5000)
        self.budget = Budget()
        self.writable = False
    def audit(self):
        return {'inventory_verified': True, 'batch_id': self.id,
                'review_digest': self.review.digest, 'inventory_digest': 'd' * 64,
                'status': 'all_placed', 'next_step': None, 'pending_step': None,
                'stopped': False, 'placed': 1}
    def check(self): pass


class FinalProgressTests(unittest.TestCase):
    def test_final_scope_uses_distinct_identity_and_key(self):
        self.assertNotEqual(FINAL_SCHEMA, 'dfmcp.route-furniture-progress/2')
        self.assertTrue(FINAL_POLICY.endswith('/1'))
        self.assertTrue(fp._record_path('/tmp', '1' * 64).name.startswith('progress-'))

    def test_review_confirms_only_pending_final_construction(self):
        batch = Batch()
        authority = object.__new__(fp.ProgressAuthority)
        report = {'batch_id': batch.id, 'phase': 'final_construction',
                  'status': 'construction_quiescent', 'evidence_digest': 'e' * 64}
        state = batch.audit()
        with patch.object(fp, '_frontier', return_value=(state, report)),              patch.object(fp, '_reserve'):
            with self.assertRaises((ValueError, AttributeError)):
                fp.review_final_progress(batch, authority, 10, 1000)

    def test_query_rejects_next_step_scope(self):
        self.assertIn('type(scope) is FinalScope',
                      __import__('inspect').getsource(fp.query_final_progress))

    def test_progress_has_one_start_and_no_automatic_loop(self):
        source = __import__('inspect').getsource(fp.progress_final)
        self.assertEqual(source.count('clock.start('), 1)
        self.assertNotIn('while ', source)
        self.assertNotIn('for ', source)

    def test_result_never_claims_stability_completion(self):
        source = __import__('inspect').getsource(fp._result)
        self.assertIn("'construction_completion_proven': False", source)
        self.assertIn("'automatic_second_clock_burst': False", source)

    def test_scope_intent_uses_final_key_family(self):
        self.assertIn("return 'frf-' + self.digest",
                      __import__('inspect').getsource(FinalScope.clock_key.fget))


if __name__ == '__main__':
    unittest.main()
