"""Read-only final-inspection routing/CLI using explicit infrastructure ports."""
import json
import os
import unittest
from unittest.mock import patch
from test_progress_family_routing import environment, require


class FinalObserveTests(unittest.TestCase):
    def setUp(self):
        self.p, self.f, self.back = self.enterContext(environment())
        self.batch = self.back.batch
        self.route = self.p.route
        self.env = {key: value for key, value in os.environ.items() if key not in self.p.RUN_ENV}
        self.enterContext(patch.dict(os.environ, self.env, clear=True))
        self.valid = True
        self.loads = 0

        def load(cls):
            self.loads += 1
            require(not any(key in os.environ for key in self.p.RUN_ENV), 'isolated read environment')
            return cls(self.batch.address, b'b'*32, b'o'*32, b'm'*32, False)

        def guard(authority, operation='QueryPlacement'):
            require(self.valid and operation == 'QueryPlacement', 'read permission changed')

        self.enterContext(patch.object(self.route.Authority, 'load', classmethod(load), create=True))
        self.enterContext(patch.object(self.route.Authority, 'guard', guard, create=True))
        self.enterContext(patch.object(self.p.ProgressAuthority, 'load',
                                      side_effect=AssertionError('clock credentials requested')))
        self.authority = self.route.Authority.load()
        self.loads = 0

    def args(self):
        return ['observe-final', '--directory', '/private/batch', '--batch-id', self.batch.id]

    def cli(self, args=None):
        output = []
        with patch.object(self.p, 'publish', side_effect=lambda raw: output.append(json.loads(raw)) or 0):
            code = self.p.main(self.args() if args is None else args)
        return code, output

    def test_cli_works_without_run_credentials_confirmation_or_clock_directory(self):
        code, output = self.cli()
        self.assertEqual(code, 0)
        self.assertEqual(self.loads, 1)
        self.assertEqual(len(output), 1)
        self.assertTrue(output[0]['ok'])
        self.assertEqual(self.back.reads, ['final'])
        self.assertEqual(self.back.calls, [])
        self.assertEqual(self.back.records, {})
        self.assertTrue(self.batch.closed)

    def test_all_findings_remain_sampled_read_only_reports(self):
        for finding in ('construction_pending', 'construction_quiescent',
                        'construction_blocked', 'construction_unestablished'):
            with self.subTest(finding=finding):
                self.batch.finding = finding
                out = self.p.observe_final(self.batch, self.authority)
                turn = self.p.packet('observe-final', out)['agent_turn']
                self.assertEqual(turn['briefing']['final_construction_status'], finding)
                self.assertEqual(turn['briefing']['all_original_furnishings_quiescent_now'],
                                 finding == 'construction_quiescent')
                self.assertFalse(turn['briefing']['construction_completion_proven'])
                self.assertFalse(turn['briefing']['clock_commit_invoked_this_call'])
                self.assertFalse(out['advance_allowed'])
                self.assertIsNone(out['expected_plan'])
                self.assertIsNone(out['confirm_review'])
                self.assertEqual(len(out['steps']), 2)
        self.assertEqual(self.back.calls, [])

    def test_clock_and_paging_options_are_rejected_before_custody(self):
        cases = [('--ticks', '1'), ('--wall-ms', '1'), ('--confirm-progress', 'e'*64),
                 ('--clock-directory', '/private/clock'), ('--wait-ms', '0'),
                 ('--refresh-frontier',), ('--selection', 'all'), ('--limit', '1'),
                 ('--continuation', 'token')]
        for extra in cases:
            with self.subTest(extra=extra), patch.object(self.p, 'Batch', side_effect=AssertionError('opened')):
                code, output = self.cli(self.args()+list(extra))
                self.assertEqual(code, 2)
                self.assertEqual(len(output), 1)
                self.assertFalse(output[0]['ok'])
        self.assertEqual(self.back.reads, [])
        self.assertEqual(self.loads, 0)

    def test_incomplete_or_stopped_batch_never_becomes_empty_success(self):
        for status, stopped in (('ready', False), ('all_placed', True)):
            self.batch.status, self.batch.stopped = status, stopped
            with self.assertRaises(ValueError):
                self.p.observe_final(self.batch, self.authority)
        self.assertEqual(self.back.reads, [])

    def test_clock_authority_cannot_be_substituted_for_read_profile(self):
        clock_authority = self.p.ProgressAuthority(self.batch.address, b'b'*32, b'o'*32, b'm'*32, False, b'r'*32)
        with self.assertRaises(ValueError):
            self.p.observe_final(self.batch, clock_authority)
        self.assertEqual(self.back.reads, [])

    def test_failed_acquisition_is_not_retried(self):
        import sys
        final = sys.modules['route_furniture_final']
        with patch.object(final, 'read_frontier', side_effect=OSError('lost read')) as read:
            code, output = self.cli()
        self.assertEqual(read.call_count, 1)
        self.assertEqual(code, 2)
        self.assertFalse(output[0]['ok'])
        self.assertEqual(self.back.calls, [])

    def test_read_permission_revoked_during_observation_refuses_publication(self):
        import sys
        final = sys.modules['route_furniture_final']
        original = final.read_frontier
        def changed(*args):
            out = original(*args)
            self.valid = False
            return out
        with patch.object(final, 'read_frontier', side_effect=changed), self.assertRaises(ValueError):
            self.p.observe_final(self.batch, self.authority)

    def test_changed_original_custody_after_observation_refuses(self):
        import sys
        final = sys.modules['route_furniture_final']
        original = final.read_frontier
        def changed(*args):
            out = original(*args)
            self.batch.corrupt = True
            return out
        with patch.object(final, 'read_frontier', side_effect=changed), self.assertRaises(ValueError):
            self.p.observe_final(self.batch, self.authority)

    def test_oversize_complete_observation_is_not_truncated(self):
        self.back.padding = 'x' * self.p.MAX_OUTPUT
        code, output = self.cli()
        self.assertEqual(code, 2)
        self.assertFalse(output[0]['ok'])
        self.assertIsNone(output[0]['result'])
        self.assertEqual(self.back.records, {})

    def test_short_output_has_one_attempt_while_original_batch_is_open(self):
        attempts = []
        def short(raw):
            self.assertFalse(self.batch.closed)
            attempts.append(raw)
            return 2
        with patch.object(self.p, 'publish', side_effect=short):
            code = self.p.main(self.args())
        self.assertEqual(code, 2)
        self.assertEqual(len(attempts), 1)
        self.assertTrue(self.batch.closed)
        self.assertEqual(self.back.reads, ['final'])

    def test_wrong_final_report_identity_or_phase_is_refused(self):
        import sys
        final = sys.modules['route_furniture_final']
        original = final.read_frontier
        for field, value in (('batch_id', 'f'*64), ('phase', 'next_step')):
            def changed(*args):
                state, report = original(*args)
                report[field] = value
                return state, report
            with self.subTest(field=field), patch.object(final, 'read_frontier', side_effect=changed), self.assertRaises(ValueError):
                self.p.observe_final(self.batch, self.authority)


if __name__ == '__main__':
    unittest.main()
