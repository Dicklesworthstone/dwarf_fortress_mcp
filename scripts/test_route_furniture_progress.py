"""The original eight progress scenarios, now executed on actual clock I/O/files.

v2 requires an exact observed clock and durable batch association. Route/custody
remain explicit ports; the native clock implementation is not replaced.
"""
from pathlib import Path
import unittest

import route_furniture_clock_context as context
from test_route_furniture_clock_context import fixture, reviewed


class ProgressTests(unittest.TestCase):
    def test_review_confirms_only_current_construction_pending_frontier(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            result = m.review_progress(batch, auth, 30, 50)
            self.assertEqual(len(result['progress_review']['confirm_progress']), 64)
            batch.route_status = 'ready'
            before = len(peer.operations)
            result = m.review_progress(batch, auth, 30, 50)
            self.assertIsNone(result['progress_review']['confirm_progress'])
            self.assertEqual(len(peer.operations), before)

    def test_confirmation_binds_ticks_wall_inventory_and_frontier(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            result = m.review_progress(batch, auth, 30, 50)
            original = context.Scope.create(result['progress_review']['scope'])
            for key, value in (('game_ticks', 31), ('wall_ms', 51),
                               ('inventory_digest', 'e' * 64), ('frontier_evidence', 'f' * 64)):
                changed = context.Scope.create({**original.value(), key: value})
                self.assertNotEqual(original.digest, changed.digest)
                self.assertNotEqual(original.intent()['idempotency_key'], changed.intent()['idempotency_key'])

    def test_same_confirmation_has_one_derived_record_and_key(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            confirmation = '1' * 64
            self.assertEqual(m._clock_key(confirmation), 'frp-' + confirmation)
            self.assertEqual(m._record_path('/private/clock', confirmation),
                             Path('/private/clock/progress-' + confirmation + '.run'))

    def test_stale_confirmation_refuses_before_clock_start(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            confirmation = reviewed(m, batch, auth, 30, 50)
            with self.assertRaises(ValueError): m.progress(batch, auth, 31, 50, confirmation, str(folder))
            self.assertNotIn('PrepareRun', peer.operations)
            self.assertNotIn('CommitRun', peer.operations)
            self.assertEqual(list(folder.iterdir()), [])

    def test_query_path_is_read_only_with_respect_to_run_dispatch(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            confirmation = reviewed(m, batch, auth)
            peer.drop_commit = True
            with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, confirmation, str(folder))
            before = len(peer.operations)
            result = m.query_progress(batch, auth, confirmation, str(folder))
            self.assertEqual(peer.operations[before:], ['Handshake', 'QueryRun'])
            self.assertTrue(result['construction_progress']['clock_terminal_pause_verified'])

    def test_progress_has_exactly_one_start_and_no_loop(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            confirmation = reviewed(m, batch, auth)
            result = m.progress(batch, auth, 20, 50, confirmation, str(folder))
            self.assertEqual(peer.operations.count('PrepareRun'), 1)
            self.assertEqual(peer.operations.count('CommitRun'), 1)
            self.assertFalse(result['construction_progress']['automatic_second_clock_burst'])

    def test_progress_refuses_writable_route_batch(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            batch.writable = True
            with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, '1' * 64, str(folder))
            self.assertEqual(peer.operations, [])
            self.assertEqual(list(folder.iterdir()), [])

    def test_policy_never_claims_placement_or_automatic_second_burst(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            confirmation = reviewed(m, batch, auth)
            result = m.progress(batch, auth, 20, 50, confirmation, str(folder))
            for key in ('automatic_second_clock_burst', 'placement_dispatched',
                        'placement_retry_permitted', 'construction_completion_proven'):
                self.assertFalse(result['construction_progress'][key])


if __name__ == '__main__':
    unittest.main()
