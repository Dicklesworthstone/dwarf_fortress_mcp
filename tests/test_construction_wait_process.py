"""Actual journal/TCP/CLI foreground waits, with a joined protocol double.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from construction_plan import Goal
from construction_plan_rpc import Budget
from construction_plan_store import Journal, replay
import track_construction_plan as cli
import construction_wait as wait
from construction_wait_peer import Peer, capture, records

ROOT = Path(__file__).resolve().parents[1]


class ProcessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.journal = self.root / 'monitor'

    def create(self, selected, peer, **kwargs):
        goal = Goal(tuple(record.raw for record in selected), kwargs.pop('deadline', 10000), **kwargs)
        with Journal(str(self.journal), Budget(60000), writable=True, create=(goal, peer.address)):
            pass
        return goal

    def command(self, peer=None, *extra, operation='wait'):
        env = peer.environment() if peer else {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        p = subprocess.run([sys.executable, str(ROOT / 'scripts/track_construction_plan.py'),
                            operation, '--journal', str(self.journal), '--timeout-ms', '60000', *extra],
                           env=env, capture_output=True, timeout=30)
        self.assertEqual(p.stderr, b'')
        self.assertLessEqual(len(p.stdout), 65536)
        return p.returncode, json.loads(p.stdout)

    def state(self):
        return replay(self.journal.read_bytes(), Budget(60000))

    def test_complete_32_target_plan_waits_through_real_paging_and_replays_offline(self):
        selected = records(32)
        raws = [capture(selected, tick, filler=2000) for tick in (101, 102)]
        self.assertGreater(len(raws[0]), 65536)
        with Peer(selected, raws) as peer:
            goal = self.create(selected, peer)
            code, result = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 0, result)
            self.assertEqual(result['result']['wait']['samples_published'], 2)
            self.assertEqual(result['result']['wait']['stop_reason'], 'terminal')
            self.assertEqual(result['result']['progress']['phase'], 'satisfied')
            self.assertEqual(len(result['result']['targets']), 32)
            self.assertEqual(result['result']['identity']['goal_digest'], goal.digest)
            self.assertEqual(result['agent_turn']['active_work'], [])
            self.assertEqual(peer.connections, 2)
        before = self.journal.read_bytes()
        code, terminal = self.command()
        self.assertEqual(code, 0, terminal)
        self.assertEqual(terminal['result']['wait']['samples_published'], 0)
        self.assertFalse(terminal['result']['native_contacted'])
        self.assertEqual(self.journal.read_bytes(), before)
        self.assertEqual(self.state().progress.phase, 'satisfied')

    def test_sample_slice_resumes_same_goal_without_renewing_observation_allowance(self):
        selected = records(2)
        with Peer(selected, [capture(selected, t) for t in (101, 102, 103)]) as peer:
            goal = self.create(selected, peer, stable_samples=3)
            for expected in (1, 2, 3):
                code, result = self.command(peer, '--wait-samples', '1')
                self.assertEqual(code, 0, result)
                self.assertEqual(result['result']['identity']['goal_digest'], goal.digest)
                self.assertEqual(result['result']['progress']['observations'], expected)
                self.assertEqual(result['result']['wait']['samples_published'], 1)
            self.assertEqual(result['result']['progress']['phase'], 'satisfied')
            self.assertEqual(peer.connections, 3)

    def test_deadline_and_total_observation_limit_remain_terminal(self):
        selected = records()
        for policy, ticks, reason in (({'deadline': 110}, (110,), 'game_deadline_reached'),
                                     ({'max_observations': 2}, (101, 102), 'sample_budget_exhausted')):
            with self.subTest(reason=reason), tempfile.TemporaryDirectory(dir=self.root) as directory:
                self.journal = Path(directory) / 'monitor'
                with Peer(selected, [capture(selected, tick, stages=[0]) for tick in ticks]) as peer:
                    self.create(selected, peer, **policy)
                    code, value = self.command(peer, '--poll-ms', '10')
                    self.assertEqual(code, 0, value)
                    self.assertEqual(value['result']['progress']['phase'], 'expired')
                    self.assertEqual(value['result']['progress']['reason'], reason)
                    self.assertEqual(peer.connections, len(ticks))

    def test_nonadvancing_ticks_stop_without_satisfaction_or_unpause(self):
        selected = records()
        with Peer(selected, [capture(selected, 101)] * 3) as peer:
            self.create(selected, peer)
            code, value = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 0, value)
            self.assertEqual(value['result']['wait']['stop_reason'], 'game_tick_not_advanced')
            self.assertEqual(value['result']['progress']['streak'], 1)
            self.assertFalse(value['result']['progress']['sampled_condition_satisfied'])
            self.assertEqual(peer.connections, 3)

    def test_lost_reply_preserves_intent_then_explicit_wait_restarts_stability(self):
        selected = records(2)
        with Peer(selected, [capture(selected, t) for t in (101, 102, 103, 104)],
                  faults={1: 'lost_final_receipt'}) as peer:
            goal = self.create(selected, peer)
            code, failed = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 2)
            self.assertFalse(failed['ok'])
            self.assertEqual(peer.connections, 2)
            state = self.state()
            self.assertTrue(state.progress.reading)
            self.assertEqual(state.progress.observations, 1)
            code, recovered = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 0, recovered)
            self.assertEqual(peer.connections, 4)
            self.assertEqual(recovered['result']['identity']['goal_digest'], goal.digest)
            self.assertEqual(recovered['result']['progress']['interrupted_reads'], 1)
            self.assertEqual(recovered['result']['progress']['phase'], 'satisfied')
            self.assertEqual(recovered['result']['progress']['first_tick'], 103)

    def test_bad_digest_and_release_are_not_automatically_retried(self):
        selected = records()
        for fault in ('bad_digest', 'bad_release', 'lost_release'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory(dir=self.root) as directory:
                self.journal = Path(directory) / 'monitor'
                with Peer(selected, [capture(selected, 101)], faults={0: fault}) as peer:
                    self.create(selected, peer)
                    code, value = self.command(peer)
                    self.assertEqual(code, 2, value)
                    self.assertEqual(peer.connections, 1)
                    self.assertTrue(self.state().progress.reading)
                    self.assertEqual(self.state().progress.observations, 0)

    def test_changed_source_and_removal_stop_whole_plan(self):
        selected = records(2)
        cases = [(dict(generations={1: 12}), [capture(selected, 101), capture(selected, 102)], 'invalidated'),
                 ({}, [capture(selected, 101, removal=True)], 'failed')]
        for options, raws, phase in cases:
            with self.subTest(phase=phase), tempfile.TemporaryDirectory(dir=self.root) as directory:
                self.journal = Path(directory) / 'monitor'
                with Peer(selected, raws, **options) as peer:
                    self.create(selected, peer)
                    code, value = self.command(peer, '--poll-ms', '10')
                    self.assertEqual(code, 0, value)
                    self.assertEqual(value['result']['progress']['phase'], phase)
                    self.assertEqual(value['result']['wait']['stop_reason'], 'terminal')
                    self.assertEqual(peer.connections, len(raws))

    def test_rpc_budget_returns_complete_partial_wait_without_new_intent(self):
        selected = records(32)
        with Peer(selected, [capture(selected, t) for t in (101, 102, 103, 104)]) as peer:
            self.create(selected, peer, stable_samples=5)
            code, value = self.command(peer, '--poll-ms', '10')
            self.assertEqual(code, 0, value)
            self.assertEqual(value['result']['wait']['stop_reason'], 'rpc_allowance')
            self.assertEqual(value['result']['wait']['samples_published'], 4)
            self.assertEqual(value['result']['progress']['observations'], 4)
            self.assertFalse(value['result']['progress']['read_outcome_unknown'])
            self.assertEqual(len(value['result']['targets']), 32)
            self.assertTrue(value['agent_turn']['active_work'])
            self.assertEqual(peer.connections, 4)

    def test_revoked_authority_during_delay_prevents_second_connection(self):
        selected = records()
        with Peer(selected, [capture(selected, 101)]) as peer:
            self.create(selected, peer)
            def revoke(_):
                os.environ['DFMCP_BUILD_TOKEN'] = 'r' * 40
            stdout = io.StringIO()
            with patch.dict(os.environ, peer.environment(), clear=True), patch.object(wait.time, 'sleep', revoke), redirect_stdout(stdout):
                code = cli.main(['wait', '--journal', str(self.journal), '--timeout-ms', '60000'])
            self.assertEqual(code, 2)
            self.assertFalse(json.loads(stdout.getvalue())['ok'])
            self.assertEqual(peer.connections, 1)
            self.assertEqual(self.state().progress.observations, 1)
            self.assertFalse(self.state().progress.reading)

    def test_final_render_revocation_withholds_completed_cached_evidence(self):
        selected = records()
        with Peer(selected, [capture(selected, 101), capture(selected, 102)]) as peer:
            self.create(selected, peer)
            original_output = cli.output
            def output(value):
                raw = original_output(value)
                view = value['result'].get('wait')
                if view is not None and view['stop_reason'] == 'terminal':
                    os.environ['DFMCP_BUILD_TOKEN'] = 'r' * 40
                return raw
            stdout = io.StringIO()
            with patch.dict(os.environ, peer.environment(), clear=True), patch.object(cli, 'output', output), \
                 patch.object(wait.time, 'sleep', lambda _: None), redirect_stdout(stdout):
                code = cli.main(['wait', '--journal', str(self.journal), '--timeout-ms', '60000'])
            self.assertEqual(code, 2)
            value = json.loads(stdout.getvalue())
            self.assertIsNone(value['result']['progress'])
            self.assertFalse(value['result']['custody_verified'])
            self.assertEqual(self.state().progress.phase, 'satisfied')

    def test_wait_never_creates_or_retargets_a_monitor(self):
        code, value = self.command(None)
        self.assertEqual(code, 2)
        self.assertFalse(self.journal.exists())
        selected = records()
        with Peer(selected, []) as peer:
            goal = self.create(selected, peer)
            original = self.journal.read_bytes()
            for option, arg in (('--deadline-tick', '20000'), ('--stable-samples', '5'),
                                ('--receipts-file', '/not/a/receipt'), ('--wait-samples', '0')):
                code, _ = self.command(peer, option, arg)
                self.assertEqual(code, 2)
                self.assertEqual(self.journal.read_bytes(), original)
            self.assertEqual(peer.connections, 0)
            self.assertEqual(self.state().goal.digest, goal.digest)

    def test_existing_start_sample_inspect_and_cancel_workflow_is_unchanged(self):
        selected = records()
        bundle = self.root / 'receipts.json'
        bundle.write_text(json.dumps({'schema': cli.RECEIPT_SCHEMA,
                                      'receipts': [{'canonical_record_hex': selected[0].raw.hex()}]}))
        bundle.chmod(0o600)
        with Peer(selected, [capture(selected, 101, stages=[0]), capture(selected, 102, stages=[0])]) as peer:
            code, started = self.command(peer, '--receipts-file', str(bundle), '--deadline-tick', '10000', operation='start')
            self.assertEqual(code, 0, started)
            self.assertNotIn('wait', started['result'])
            code, sampled = self.command(peer, operation='sample')
            self.assertEqual(code, 0, sampled)
            self.assertEqual(sampled['result']['progress']['observations'], 2)
            self.assertEqual(peer.connections, 2)
        code, inspected = self.command(operation='inspect')
        self.assertEqual(code, 0, inspected)
        code, cancelled = self.command(operation='cancel')
        self.assertEqual(code, 0, cancelled)
        self.assertEqual(cancelled['result']['progress']['phase'], 'cancelled')
        code, terminal = self.command()
        self.assertEqual(code, 0, terminal)
        self.assertEqual(terminal['result']['wait']['samples_published'], 0)


if __name__ == '__main__':
    unittest.main()
