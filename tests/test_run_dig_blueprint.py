"""Actual legacy dig coordinator, private custody, codec and joined TCP peer.

The peer is synthetic, not a native DFHack producer. No production admission
or excavation completion is inferred. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from contextlib import ExitStack
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import dig_blueprint as b
import dig_blueprint_client as bp
import dig_designation_client as d
import run_dig_blueprint as run
from dig_prefix_peer import DigPeer, TOKEN, blueprint


class PrefixRunTests(unittest.TestCase):
    def setUp(self):
        self.resources = ExitStack()
        self.addCleanup(self.resources.close)
        self.directory = Path(self.resources.enter_context(tempfile.TemporaryDirectory()))
        self.root = self.directory / 'batch'
        self.root.mkdir(mode=0o700)
        self.peer = self.resources.enter_context(DigPeer())
        env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        env.update(DFMCP_ALLOW_UNADMITTED_DIG_V1_16='1', DFMCP_DIG_ALLOW_DESIGNATE='1',
                   DFMCP_DIG_ENDPOINT=self.peer.address, DFMCP_DIG_TOKEN=TOKEN)
        self.resources.enter_context(patch.dict(os.environ, env, clear=True))

    def initialize(self, count=3, value=None):
        self.layout = b.Layout.from_json(blueprint(count) if value is None else value)
        initialized = bp.initialize(self.root, self.layout, 'region1', 2, False,
                                    bp.POLICY, bp.Budget(10000))
        self.batch_id = initialized['batch_id']

    def review(self, count=2):
        return run.review(self.root, count, run.Budget(10000, count))

    def execute(self, reviewed, count=None, budget=None):
        count = reviewed['review']['step_count'] if count is None else count
        return run.run(self.root, self.batch_id, count, reviewed['review']['confirm_run'],
                       run.Budget(10000, count) if budget is None else budget)

    def inventory(self):
        return bp.inspect(self.root, bp.Budget(10000))

    def files(self):
        return {str(p.relative_to(self.root)): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}

    def cli(self, *args, env=None):
        return subprocess.run([sys.executable, str(SCRIPTS / 'run_dig_blueprint.py'), *args,
            '--directory', str(self.root)], capture_output=True, timeout=30, env=env)

    def assert_no_recovery(self):
        self.assertFalse(any(name in ('QueryDesignation', 'CancelDesignation') for _, name in self.peer.calls))
        self.assertTrue(all(count <= 1 for count in self.peer.connection_commits))

    def test_two_steps_execute_real_one_shot_connections_and_original_custody(self):
        self.initialize()
        reviewed = self.review()
        result = self.execute(reviewed)
        self.assertTrue(result['ok'])
        self.assertEqual(result['run']['stop_reason'], 'prefix_completed')
        self.assertEqual(result['run']['connections_started'], 2)
        self.assertEqual(result['run']['rpc_calls_used'], 22)
        self.assertEqual(result['batch']['designated_steps'], 2)
        self.assertEqual(result['batch']['next_step'], 2)
        self.assertEqual(result['blueprint'], self.layout.blueprint())
        self.assertEqual(self.peer.commits, [s['key'] for s in reviewed['review']['steps']])
        self.assertEqual(len(list((self.root / 'effects').glob('step-*.json'))), 2)
        self.assertEqual(len(list((self.root / 'effects').glob('.dfmcp-dig-terminal-*.json'))), 2)
        self.assertFalse(result['excavation_completion_proven'])
        self.assert_no_recovery()

    def test_all_512_tiles_in_eight_native_steps_complete_without_hull_writes(self):
        value = {'schema': b.SCHEMA, 'parts': [{'region': {'origin': [4, 4, 2],
                 'size': [32, 16, 1]}, 'shape': 'floor'}]}
        self.initialize(value=value)
        self.assertEqual(len(self.layout.rectangles), 8)
        reviewed = self.review(8)
        result = self.execute(reviewed)
        self.assertTrue(result['ok'])
        self.assertEqual(result['run']['stop_reason'], 'all_designations_verified')
        self.assertEqual(result['batch']['designated_steps'], 8)
        self.assertEqual(self.peer.designated, set(self.layout.targets))
        self.assertEqual(result['run']['rpc_calls_used'], 88)
        self.assertLessEqual(len(d.canonical(result)), bp.MAX_OUTPUT)
        self.assert_no_recovery()

    def test_review_is_offline_and_cannot_change_original_files(self):
        self.initialize()
        before, calls = self.files(), list(self.peer.calls)
        with patch.dict(os.environ, {}, clear=True):
            first, second = self.review(), self.review()
        self.assertEqual(first, second)
        self.assertEqual(self.files(), before)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(first['run']['connections_started'], 0)

    def test_stale_prefix_replay_is_refused_before_any_connection(self):
        self.initialize(3)
        old = self.review(1)
        self.execute(old)
        calls = list(self.peer.calls)
        with self.assertRaises(ValueError):
            self.execute(old)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(len(self.peer.commits), 1)
        next_review = self.review(2)
        self.assertEqual(next_review['review']['start_step'], 1)
        self.assertTrue(self.execute(next_review)['ok'])
        self.assertEqual(len(self.peer.commits), 3)

    def test_bad_confirmation_and_changed_count_have_no_native_or_disk_effect(self):
        self.initialize()
        reviewed = self.review(2)
        before, calls = self.files(), list(self.peer.calls)
        with self.assertRaises(ValueError):
            run.run(self.root, self.batch_id, 2, '0' * 64, run.Budget(10000, 2))
        with self.assertRaises(ValueError):
            self.execute(reviewed, count=1)
        self.assertEqual(self.files(), before)
        self.assertEqual(self.peer.calls, calls)

    def test_later_blocked_target_stops_without_skipping_or_creating_its_intent(self):
        self.initialize()
        reviewed = self.review()
        self.peer.block_region = self.layout.regions()[1]
        result = self.execute(reviewed)
        self.assertFalse(result['ok'])
        self.assertEqual(result['run']['stop_reason'], 'preflight_blocked')
        self.assertEqual(result['batch']['designated_steps'], 1)
        self.assertEqual(result['batch']['next_step'], 1)
        self.assertEqual(result['run']['outcomes'][-1]['state'], 'not_started')
        self.assertIn('occupied_or_job', result['run']['outcomes'][-1]['blockers'])
        self.assertEqual(result['run']['steps_considered'], 2)
        self.assertEqual(result['run']['steps_recorded'], 1)
        self.assertFalse((self.root / 'effects' / 'step-001.json').exists())
        self.assertEqual(len(self.peer.commits), 1)
        self.assert_no_recovery()

    def test_lost_commit_keeps_exact_unknown_key_and_never_contacts_next_step(self):
        self.initialize()
        reviewed = self.review()
        self.peer.mode, self.peer.at_step = 'drop_commit', 1
        with self.assertRaises((OSError, ValueError)):
            self.execute(reviewed)
        retained = self.inventory()
        self.assertEqual(retained['phase'], 'unresolved')
        self.assertEqual(retained['designated_steps'], 1)
        self.assertEqual(retained['unresolved_step'], 1)
        self.assertEqual(len(self.peer.commits), 2)
        self.assertEqual(retained['records'][-1]['key'], reviewed['review']['steps'][1]['key'])
        self.assertFalse((self.root / 'effects' / 'step-002.json').exists())
        calls = list(self.peer.calls)
        with self.assertRaises(ValueError):
            self.review(1)
        self.assertEqual(self.peer.calls, calls)
        self.assert_no_recovery()
        # Explicit existing recovery can acknowledge the retained native result,
        # but must never commit again or automatically begin the next step.
        self.peer.mode = None
        recovered = bp.recover(self.root, self.batch_id, 1, False, bp.Budget(10000))
        self.assertEqual(recovered['batch']['designated_steps'], 2)
        self.assertEqual(len(self.peer.commits), 2)
        self.assertEqual(self.inventory()['next_step'], 2)

    def test_lost_prepare_blocks_new_run_without_any_commit(self):
        self.initialize()
        reviewed = self.review()
        self.peer.mode = 'drop_prepare'
        with self.assertRaises((OSError, ValueError)):
            self.execute(reviewed)
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assertEqual(self.peer.commits, [])
        self.assert_no_recovery()

    def test_replayed_preparation_never_gets_a_commit(self):
        self.initialize()
        self.peer.mode = 'prepared_replay'
        result = self.execute(self.review())
        self.assertFalse(result['ok'])
        self.assertEqual(result['run']['stop_reason'], 'indeterminate')
        self.assertEqual(self.peer.commits, [])
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assert_no_recovery()

    def test_native_unknown_ack_stops_the_whole_prefix(self):
        self.initialize()
        self.peer.mode = 'unknown'
        result = self.execute(self.review())
        self.assertEqual(result['run']['stop_reason'], 'indeterminate')
        self.assertEqual(result['batch']['unresolved_step'], 0)
        self.assertEqual(len(self.peer.commits), 1)
        self.assert_no_recovery()

    def test_terminal_native_refusal_is_not_a_reason_to_skip_original_work(self):
        self.initialize()
        self.peer.mode = 'refuse'
        result = self.execute(self.review())
        self.assertEqual(result['run']['stop_reason'], 'native_refused')
        self.assertFalse(result['ok'])
        self.assertEqual(result['batch']['phase'], 'refused')
        self.assertEqual(len(self.peer.commits), 1)
        self.assertEqual(self.peer.designated, set())
        with self.assertRaises(ValueError):
            self.review(1)

    def test_lost_preflight_read_creates_no_intent_and_no_retry(self):
        self.initialize()
        self.peer.mode = 'drop_read'
        before = self.files()
        with self.assertRaises((OSError, ValueError)):
            self.execute(self.review())
        self.assertEqual(self.files(), before)
        self.assertEqual(self.peer.commits, [])
        self.assert_no_recovery()

    def test_changed_revalidation_read_refuses_before_write_ahead_intent(self):
        self.initialize()
        self.peer.mode = 'change_read'
        before = self.files()
        with self.assertRaisesRegex(ValueError, 'terrain changed'):
            self.execute(self.review())
        self.assertEqual(self.files(), before)
        self.assertEqual(self.peer.commits, [])

    def test_corrupt_receipt_leaves_unresolved_effect_instead_of_success(self):
        self.initialize()
        self.peer.mode = 'bad_receipt'
        with self.assertRaises(ValueError):
            self.execute(self.review())
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assertEqual(len(self.peer.commits), 1)
        self.assert_no_recovery()

    def test_source_generation_change_stops_before_preparation(self):
        self.initialize()
        self.peer.mode = 'generation'
        with self.assertRaises(ValueError):
            self.execute(self.review())
        self.assertEqual(self.peer.commits, [])
        self.assertFalse(list((self.root / 'effects').glob('step-*.json')))

    def test_tick_regression_after_one_effect_does_not_start_another_intent(self):
        self.initialize()
        def regress(operation, peer):
            if len(peer.commits) == 1 and operation == 'ReadDesignation':
                peer.tick = 99
        self.peer.on_operation = regress
        with self.assertRaises(ValueError):
            self.execute(self.review())
        self.assertEqual(len(self.peer.commits), 1)
        self.assertEqual(self.inventory()['next_step'], 1)

    def test_shared_rpc_and_wire_allowances_never_reset_per_step(self):
        self.initialize()
        reviewed = self.review()
        budget = run.Budget(10000, 2)
        budget.calls_left = run.RPC_PER_STEP
        with self.assertRaisesRegex(ValueError, 'shared allowance'):
            self.execute(reviewed, budget=budget)
        self.assertEqual(len(self.peer.commits), 1)
        self.assertEqual(budget.calls_left, 0)
        self.assertEqual(budget.connections_started, 1)
        second = self.review(2)
        budget = run.Budget(10000, 2)
        budget.network_left = run.WIRE_PER_STEP
        with self.assertRaisesRegex(ValueError, 'shared allowance'):
            self.execute(second, budget=budget)
        self.assertEqual(len(self.peer.commits), 2)
        self.assertEqual(budget.connections_started, 1)

    def test_output_capacity_is_reserved_before_any_native_contact(self):
        self.initialize()
        reviewed = self.review()
        calls, files = list(self.peer.calls), self.files()
        with patch.object(run, 'ROW_RESERVATION', bp.MAX_OUTPUT), self.assertRaisesRegex(ValueError, 'cannot fit'):
            self.execute(reviewed)
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(self.files(), files)

    def test_disabling_designation_or_changing_profile_prevents_contact(self):
        self.initialize()
        reviewed = self.review()
        calls = list(self.peer.calls)
        os.environ.pop('DFMCP_DIG_ALLOW_DESIGNATE')
        with self.assertRaises(ValueError):
            self.execute(reviewed)
        os.environ['DFMCP_DIG_ALLOW_DESIGNATE'] = '1'
        os.environ['DFMCP_MAP_TOKEN'] = 'foreign-profile'
        with self.assertRaises(ValueError):
            self.execute(reviewed)
        self.assertEqual(self.peer.calls, calls)

    def test_authority_revocation_after_preparation_preserves_unknown_original_intent(self):
        self.initialize()
        def revoke(operation, peer):
            if operation == 'PrepareDesignation':
                os.environ.pop('DFMCP_DIG_ALLOW_DESIGNATE', None)
        self.peer.on_operation = revoke
        with self.assertRaises(ValueError):
            self.execute(self.review())
        self.assertEqual(self.peer.commits, [])
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assert_no_recovery()

    def test_keyboard_interrupt_after_intent_sync_never_dispatches(self):
        self.initialize()
        reviewed = self.review()
        def interrupt(client, intent):
            self.assertTrue((self.root / 'effects' / 'step-000.json').is_file())
            raise KeyboardInterrupt()
        with patch.object(d.Client, 'prepare', interrupt), self.assertRaises(KeyboardInterrupt):
            self.execute(reviewed)
        self.assertEqual(self.peer.commits, [])
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assert_no_recovery()

    def test_intent_sync_failure_retains_orphan_and_blocks_restart(self):
        self.initialize()
        reviewed = self.review()
        fsync = os.fsync
        def fail_intent(fd):
            if os.readlink(f'/proc/self/fd/{fd}').endswith('step-000.json'):
                raise OSError('injected original intent sync failure')
            return fsync(fd)
        with patch.object(os, 'fsync', fail_intent), self.assertRaises(OSError):
            self.execute(reviewed)
        self.assertEqual(self.peer.commits, [])
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        with self.assertRaises(ValueError):
            self.review(1)

    def test_lost_terminal_sync_stops_even_when_receipt_can_later_be_reverified(self):
        self.initialize()
        reviewed = self.review()
        fsync = os.fsync
        def fail_receipt(fd):
            if '.dfmcp-dig-terminal-' in os.readlink(f'/proc/self/fd/{fd}'):
                raise OSError('injected terminal sync failure')
            return fsync(fd)
        with patch.object(os, 'fsync', fail_receipt), self.assertRaises(OSError):
            self.execute(reviewed)
        self.assertEqual(len(self.peer.commits), 1)
        self.assertEqual(self.inventory()['designated_steps'], 1)
        self.assert_no_recovery()

    def test_stopped_batch_never_unlocks_future_work(self):
        self.initialize()
        reviewed = self.review(1)
        self.execute(reviewed)
        bp.stop(self.root, self.batch_id, bp.Budget(10000))
        calls = list(self.peer.calls)
        with self.assertRaises(ValueError):
            self.review(1)
        self.assertEqual(self.peer.calls, calls)

    def test_missing_prior_terminal_receipt_refuses_before_any_new_connection(self):
        self.initialize()
        self.execute(self.review(1))
        receipt = next((self.root / 'effects').glob('.dfmcp-dig-terminal-*.json'))
        receipt.unlink()  # Explicit destructive fixture: uncertain effect must not disappear.
        calls = list(self.peer.calls)
        with self.assertRaises(ValueError):
            self.review(1)
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assertEqual(self.peer.calls, calls)

    def test_final_render_revocation_withholds_success_after_durable_effects(self):
        self.initialize(2)
        reviewed = self.review()
        serialize = bp.serialize_result
        def revoke(result, budget):
            raw = serialize(result, budget)
            if result.get('operation') == 'run' and result['batch']['designated_steps'] == 2:
                os.environ.pop('DFMCP_DIG_ALLOW_DESIGNATE', None)
            return raw
        class Output:
            def __init__(self):
                self.buffer = io.BytesIO()
        output = Output()
        with patch.object(bp, 'serialize_result', revoke), patch.object(sys, 'stdout', output):
            status = run.main(['run', '--directory', str(self.root), '--steps', '2',
                '--batch-id', self.batch_id, '--confirm-run', reviewed['review']['confirm_run']])
        self.assertEqual(status, 2)
        value = json.loads(output.buffer.getvalue())
        self.assertFalse(value['ok'])
        self.assertEqual(value['effect_status'], 'unknown')
        self.assertEqual(value['agent_turn']['schema'], 'dfmcp.agent_turn/1')
        self.assertEqual(self.inventory()['designated_steps'], 2)
        self.assertEqual(len(self.peer.commits), 2)
        self.assert_no_recovery()

    def test_cli_sigkill_after_native_effect_does_not_resume_or_retry_prefix(self):
        self.initialize(3)
        reviewed = self.review(2)
        ready = threading.Event()
        killed = threading.Event()
        process = None
        def kill_after_effect(key, peer):
            if not ready.wait(5):
                raise AssertionError('owned child was not published to the kill boundary')
            process.kill()
            killed.set()
        self.peer.after_commit = kill_after_effect
        process = subprocess.Popen([sys.executable, str(SCRIPTS / 'run_dig_blueprint.py'),
            'run', '--directory', str(self.root), '--steps', '2', '--batch-id', self.batch_id,
            '--confirm-run', reviewed['review']['confirm_run']], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        ready.set()
        try:
            stdout, stderr = process.communicate(timeout=15)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        self.assertTrue(killed.is_set(), (stdout, stderr))
        self.assertEqual(process.returncode, -9)
        self.assertEqual(stdout, b'')
        self.assertEqual(len(self.peer.commits), 1)
        self.assertEqual(self.inventory()['phase'], 'unresolved')
        self.assertFalse((self.root / 'effects' / 'step-001.json').exists())
        self.assert_no_recovery()
        calls = list(self.peer.calls)
        attempted = self.cli('run', '--steps', '2', '--batch-id', self.batch_id,
                              '--confirm-run', reviewed['review']['confirm_run'])
        self.assertEqual(attempted.returncode, 2)
        self.assertEqual(self.peer.calls, calls)
        self.peer.after_commit = None
        recovered = bp.recover(self.root, self.batch_id, 0, False, bp.Budget(10000))
        self.assertEqual(recovered['batch']['designated_steps'], 1)
        self.assertEqual(len(self.peer.commits), 1)

    def test_cli_review_execute_replay_refusal_and_offline_original_inspection(self):
        self.initialize(2)
        offline_env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
        reviewed = self.cli('review', '--steps', '2', env=offline_env)
        self.assertEqual(reviewed.returncode, 0, reviewed.stderr)
        value = json.loads(reviewed.stdout)
        result = self.cli('run', '--steps', '2', '--batch-id', self.batch_id,
                          '--confirm-run', value['review']['confirm_run'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout)['run']['stop_reason'], 'all_designations_verified')
        calls = list(self.peer.calls)
        replay = self.cli('run', '--steps', '2', '--batch-id', self.batch_id,
                          '--confirm-run', value['review']['confirm_run'])
        self.assertEqual(replay.returncode, 2)
        self.assertEqual(json.loads(replay.stdout)['error']['code'], 'DIG_PREFIX_RUN_REFUSED')
        self.assertEqual(self.peer.calls, calls)
        inspected = subprocess.run([sys.executable, str(SCRIPTS / 'dig_blueprint_client.py'),
            'inspect', '--directory', str(self.root)], capture_output=True, timeout=10, env=offline_env)
        self.assertEqual(inspected.returncode, 0, inspected.stdout + inspected.stderr)
        self.assertEqual(json.loads(inspected.stdout)['designated_steps'], 2)

    def test_cli_short_write_never_repeats_the_run_or_emits_a_second_object(self):
        self.initialize(2)
        reviewed = self.review()
        class Short:
            def __init__(self):
                self.buffer = self; self.calls = 0
            def write(self, raw):
                self.calls += 1
                return 1
            def flush(self):
                pass
        out = Short()
        with patch.object(sys, 'stdout', out):
            result = run.main(['run', '--directory', str(self.root), '--steps', '2',
                '--batch-id', self.batch_id, '--confirm-run', reviewed['review']['confirm_run']])
        self.assertEqual(result, 2)
        self.assertEqual(out.calls, 1)
        self.assertEqual(len(self.peer.commits), 2)
        self.assertEqual(self.inventory()['phase'], 'designations_verified')

    def test_cli_refuses_changed_policy_flags_before_any_contact(self):
        self.initialize()
        calls = list(self.peer.calls)
        for extra in (['--allow-hidden-neighbors'], ['--checkpoint-policy', 'new'],
                      ['--confirm-run', '0' * 64], ['--steps', '9']):
            result = self.cli('review', '--steps', '2', *extra)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(json.loads(result.stdout)['ok'])
        self.assertEqual(self.peer.calls, calls)


if __name__ == '__main__':
    unittest.main()
