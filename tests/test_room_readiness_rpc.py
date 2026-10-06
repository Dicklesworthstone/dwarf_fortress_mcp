"""Actual foreground clients with a joined, fragmented three-profile peer."""
from dataclasses import replace
import os
import unittest
from unittest.mock import patch

import room_readiness as r
import room_readiness_rpc as rpc
import room_readiness_fixtures as f
from room_readiness_peer import Peer, environment


class ReadinessRPCTests(unittest.TestCase):
    def test_one_connection_exact_bracket_and_no_mutation_binding(self):
        with Peer() as peer, environment(peer):
            captured = rpc.acquire(rpc.Authority.load(), peer.goal, rpc.Budget(10000))
            self.assertEqual(peer.connections, 1)
            self.assertEqual(len(peer.binds), 6)
            self.assertFalse(any('Prepare' in name or 'Commit' in name or 'Cancel' in name for _, name in peer.binds))
            calls = peer.calls
            before, after = calls.index(('map', 'before')), calls.index(('map', 'after'))
            self.assertEqual(calls[:before].count(('build', 'QueryPlacement')), len(peer.goal.condition.receipts))
            self.assertEqual(calls[after + 1:].count(('build', 'QueryPlacement')), len(peer.goal.condition.receipts))
            self.assertIn(('operations', 'release'), calls[before:after])
            captured.validate(peer.goal, lambda: None)
            state = r.advance(r.begin_read(r.Progress(peer.goal.digest)), peer.goal, captured)
            self.assertEqual((state.phase, state.streak), ('candidate', 1))
            self.assertGreater(peer.fragments, 20)

    def test_closes_connection_before_decode_and_never_reuses_permit(self):
        with Peer() as peer, environment(peer):
            original = r.Sample.validate
            def validate(sample, goal, guard):
                self.assertTrue(peer.connection_closed.wait(2), 'socket retained through CPU projection')
                return original(sample, goal, guard)
            with patch.object(r.Sample, 'validate', validate):
                with rpc.Client(rpc.Authority.load(), peer.goal, rpc.Budget(10000)) as client:
                    client.capture_once()
                    self.assertTrue(client.closed)
                    self.assertRaises(ValueError, client.capture_once)
            self.assertEqual(peer.connections, 1)

    def test_bad_frames_source_and_release_never_return_a_sample_or_retry(self):
        for fault in ('alias_map', 'nonce_map', 'oversize_map', 'lost_map_after', 'changed_map',
                      'map_generation', 'map_tick', 'map_software', 'release', 'page_digest', 'receipt_after'):
            with self.subTest(fault=fault), Peer() as peer, environment(peer):
                peer.fault = fault
                with self.assertRaises((ValueError, OSError)):
                    rpc.acquire(rpc.Authority.load(), peer.goal, rpc.Budget(10000))
                self.assertEqual(peer.connections, 1)
                self.assertLessEqual(peer.calls.count(('map', 'before')), 1)
                self.assertLessEqual(peer.calls.count(('map', 'after')), 1)

    def test_no_unpause_or_automatic_resample_for_moving_game(self):
        with Peer() as peer, environment(peer):
            peer.ops_options = {'paused': False}
            self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), peer.goal, rpc.Budget(10000))
            self.assertEqual(peer.connections, 1)

    def test_endpoint_and_invalid_goal_rejected_before_socket(self):
        with Peer() as peer, environment(peer), patch('socket.socket', side_effect=AssertionError('unexpected socket')):
            foreign = f.goal()
            self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), foreign, rpc.Budget(10000))
            self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), object(), rpc.Budget(10000))

    def test_isolated_authority_rejects_place_grants_bad_tokens_and_unrelated_profiles(self):
        with Peer() as peer, environment(peer):
            for key, value in [('DFMCP_BUILD_ALLOW_PLACE', '1'), ('DFMCP_ALLOW_UNADMITTED_BUILD_V1_19', '1'),
                               (rpc.MAP_TOKEN, 'short'), (rpc.OPT_IN, 'true')]:
                with self.subTest(key=key), patch.dict(os.environ, {key: value}):
                    self.assertRaises(ValueError, rpc.Authority.load)
            self.assertEqual(peer.connections, 0)

    def test_authority_revocation_at_each_acquisition_phase_has_no_retry(self):
        for boundary in [('map', 'Handshake'), ('map', 'before'), ('operations', 'release'), ('map', 'after')]:
            with self.subTest(boundary=boundary), Peer() as peer, environment(peer):
                peer.callback = lambda op: os.environ.pop(rpc.OPT_IN, None) if op == boundary else None
                self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), peer.goal, rpc.Budget(10000))
                self.assertEqual(peer.connections, 1)

    def test_work_and_final_decode_authority_are_not_renewed(self):
        with Peer() as peer, environment(peer):
            original = r.Sample.validate
            def revoke(sample, goal, guard):
                result = original(sample, goal, guard)
                os.environ.pop(rpc.OPT_IN)
                return result
            with patch.object(r.Sample, 'validate', revoke):
                self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), peer.goal, rpc.Budget(10000))
            self.assertEqual(peer.connections, 1)
        with Peer() as peer, environment(peer):
            budget = rpc.Budget(10000)
            budget.work_steps = 0
            self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), peer.goal, budget)
            self.assertEqual(peer.connections, 0)

    def test_calls_and_bytes_are_shared_not_reset_for_map_reads(self):
        for field, limit in [('calls', 10), ('network_bytes', 1000)]:
            with self.subTest(field=field), Peer() as peer, environment(peer):
                budget = rpc.Budget(10000)
                setattr(budget, field, limit)
                self.assertRaises(ValueError, rpc.acquire, rpc.Authority.load(), peer.goal, budget)
                self.assertGreaterEqual(getattr(budget, field), 0)
                self.assertEqual(peer.connections, 1)

    def test_32_slots_646_exclusions_paged_inventory_and_shared_progress(self):
        with Peer(True, extra_items=2000) as peer, environment(peer):
            budget = rpc.Budget(30000)
            captured = rpc.acquire(rpc.Authority.load(), peer.goal, budget)
            self.assertEqual(len(peer.goal.room.allocation.request.excluded_items), 646)
            self.assertEqual(peer.calls.count(('build', 'QueryPlacement')), 64)
            self.assertGreater(peer.calls.count(('operations', 'ReadObservation')), 1)
            self.assertEqual(peer.calls.count(('operations', 'release')), 1)
            self.assertEqual(captured.furnishings.before_records, peer.goal.condition.receipts)
            state = r.advance(r.begin_read(r.Progress(peer.goal.digest)), peer.goal, captured)
            peer.tick += 10
            captured = rpc.acquire(rpc.Authority.load(), peer.goal, rpc.Budget(30000))
            state = r.advance(r.begin_read(state), peer.goal, captured)
            self.assertEqual(state.phase, 'satisfied')
            self.assertEqual(len(state.assessments), 32)
            self.assertFalse(state.view()['room_completion_proven'])


if __name__ == '__main__':
    unittest.main()
