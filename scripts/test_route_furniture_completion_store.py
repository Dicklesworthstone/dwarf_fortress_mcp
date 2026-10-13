"""Real private monitor files and actual transitions; original parent is injected."""
from contextlib import contextmanager
from dataclasses import replace
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import construction_monitor_store as custody
from construction_plan_rpc import Budget
import construction_wait as foreground
import route_furniture_completion_store as store
import track_route_furniture_completion as cli
from test_route_furniture_completion import fixture, Parent, sample, BATCH_ID

ADDRESS = ('127.0.0.1', 5000)


class Authority:
    address = ADDRESS
    valid = True
    def guard(self):
        if not self.valid:
            raise ValueError('revoked')


class CompletionStoreTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'completion'
        self.goal = fixture()
        self.parent = Parent(self.goal)
        self.parent.address = ADDRESS
        self.parent.close = lambda: None
        self.authority = Authority()

    def open(self, create=False, writable=True, goal=None):
        return store.Journal(str(self.path), Budget(60000), writable=writable,
            create=((goal or self.goal), ADDRESS) if create else None)

    def accept(self, owner, tick=100):
        owner.start_read()
        owner.accept(sample(owner.state.goal, tick), lambda state: cli.reserve('sample', state))

    def test_private_publish_reopen_and_two_sample_history(self):
        with self.open(True) as owner:
            self.assertEqual(os.stat(self.path).st_mode & 0o777, 0o600)
            self.accept(owner)
            self.assertEqual(owner.state.progress.phase, 'candidate')
            original = self.path.read_bytes()
        with self.open() as owner:
            self.assertEqual(owner.state.goal, self.goal)
            self.assertFalse(owner.read_owned)
            self.accept(owner, 101)
            self.assertEqual(owner.state.progress.phase, 'satisfied')
            self.assertTrue(self.path.read_bytes().startswith(original))
        with self.open(writable=False) as owner:
            cli.guard(owner, self.parent)
            self.assertEqual(owner.state.progress.phase, 'satisfied')
            self.assertFalse(owner.cancel(lambda state: None))

    def test_exclusive_create_and_lock_never_overwrite(self):
        with self.open(True):
            raw = self.path.read_bytes()
            with self.assertRaises(OSError):
                self.open()
            with self.assertRaises(OSError):
                self.open(True)
            self.assertEqual(self.path.read_bytes(), raw)

    def test_unowned_or_reopened_read_cannot_publish_and_streak_resets(self):
        with self.open(True) as owner:
            with self.assertRaises(ValueError):
                owner.accept(sample(self.goal), lambda state: None)
            self.accept(owner)
            owner.start_read()
        with self.open() as owner:
            with self.assertRaises(ValueError):
                owner.accept(sample(self.goal, 101), lambda state: None)
            self.accept(owner, 101)
            self.assertEqual(owner.state.progress.streak, 1)
            self.assertEqual(owner.state.progress.interruptions, 1)

    def test_failed_source_or_output_preserves_only_unknown_read_intent(self):
        for mode in ('source', 'render'):
            self.path = Path(self.directory.name) / mode
            with self.open(True) as owner:
                owner.start_read()
                raw = self.path.read_bytes()
                value = sample(self.goal)
                render = lambda state: None
                if mode == 'source':
                    value = replace(value, operations=replace(value.operations, generation=99))
                else:
                    def render(_):
                        raise ValueError('output budget')
                with self.assertRaises(ValueError):
                    owner.accept(value, render)
                self.assertTrue(owner.fenced)
                self.assertEqual(self.path.read_bytes(), raw)
            with self.open(writable=False) as recovered:
                self.assertTrue(recovered.state.progress.reading)
                self.assertEqual(recovered.state.progress.observations, 0)

    def test_partial_append_is_not_repaired_or_replayed(self):
        real_write = custody.write_all
        def torn(fd, raw, budget):
            real_write(fd, raw[:17], budget)
            raise OSError('torn')
        with self.open(True) as owner:
            owner.start_read()
            with patch.object(custody, 'write_all', torn), self.assertRaises(OSError):
                owner.accept(sample(self.goal), lambda state: None)
            self.assertTrue(owner.fenced)
            self.assertEqual(owner.state.progress.observations, 0)
        raw = self.path.read_bytes()
        with self.assertRaises(ValueError):
            self.open()
        self.assertEqual(self.path.read_bytes(), raw)

    def test_lost_sync_acknowledgment_does_not_publish_but_complete_bytes_replay(self):
        with self.open(True) as owner:
            owner.start_read()
            with patch.object(custody.os, 'fsync', side_effect=OSError('lost sync')), self.assertRaises(OSError):
                owner.accept(sample(self.goal), lambda state: None)
            self.assertTrue(owner.fenced)
            self.assertEqual(owner.state.progress.observations, 0)
        with self.open(writable=False) as owner:
            self.assertEqual(owner.state.progress.observations, 1)
            self.assertFalse(cli.packet('inspect', owner.state)['result']['storage_acknowledged_this_call'])

    def test_changed_backing_permanently_fences_owner(self):
        with self.open(True) as owner:
            raw = self.path.read_bytes()
            self.path.write_bytes(raw + b'x')
            with self.assertRaises(ValueError):
                owner.check()
            self.path.write_bytes(raw)
            with self.assertRaises(ValueError):
                owner.check()

    def test_symlink_hardlink_permissions_and_old_codec_refuse(self):
        with self.open(True):
            pass
        original = self.path
        alias = original.with_name('alias')
        alias.symlink_to(original)
        with self.assertRaises((ValueError, OSError)):
            store.Journal(str(alias), Budget(60000))
        alias.unlink()
        os.link(original, alias)
        with self.assertRaises(ValueError):
            self.open()
        alias.unlink()
        original.chmod(0o644)
        with self.assertRaises(ValueError):
            self.open()
        original.chmod(0o600)
        with self.assertRaises(ValueError):
            custody.Journal(str(original), Budget(60000))

    def test_final_sample_framing_boundaries_and_distributed_corruption_refuse(self):
        with self.open(True) as owner:
            owner.start_read()
            prefix = self.path.read_bytes()
            owner.accept(sample(self.goal), lambda state: None)
            full = self.path.read_bytes()
        tail = full[len(prefix):]
        prefixes = sorted(set(range(1, 74)) | set(range(len(tail) - 33, len(tail)))
                          | set(range(1, len(tail), max(1, len(tail) // 64))))
        for length in prefixes:
            raw = prefix + tail[:length]
            with self.assertRaises(ValueError):
                store.replay_reader(lambda start, size: raw[start:start + size], len(raw), Budget(60000))
        # A changed frame is refused even if it still parses as valid values.
        indices = sorted(set(range(40)) | set(range(len(tail) - 32, len(tail)))
                         | set(range(0, len(tail), max(1, len(tail) // 64))))
        for index in indices:
            raw = prefix + tail[:index] + bytes([tail[index] ^ 1]) + tail[index + 1:]
            with self.assertRaises(ValueError):
                store.replay_reader(lambda start, size: raw[start:start + size], len(raw), Budget(60000))

    def test_read_only_and_exhausted_capacity_refuse_before_acquisition(self):
        with self.open(True):
            pass
        with self.open(writable=False) as owner, patch.object(cli, 'acquire') as acquire:
            with self.assertRaises(ValueError):
                cli.sample_once(owner, self.parent, self.authority, lambda state: None)
            acquire.assert_not_called()
        with self.open() as owner:
            raw = self.path.read_bytes()
            with patch.object(owner, '_limits', return_value=(len(raw) + 10, 2, 100)):
                with self.assertRaises(ValueError):
                    owner.start_read()
            self.assertEqual(self.path.read_bytes(), raw)

    def test_revocation_or_parent_loss_after_read_cannot_publish_sample(self):
        for mode in ('authority', 'parent'):
            self.path = Path(self.directory.name) / mode
            self.authority.valid = self.parent.valid = True
            def read(authority, goal, budget):
                if mode == 'authority':
                    authority.valid = False
                else:
                    self.parent.valid = False
                return sample(goal)
            with self.open(True) as owner, patch.object(cli, 'acquire', read):
                with self.assertRaises(ValueError):
                    cli.sample_once(owner, self.parent, self.authority, lambda state: None)
                self.assertEqual(owner.state.progress.observations, 0)
            with self.open(writable=False) as owner:
                self.assertTrue(owner.state.progress.reading)

    def test_foreground_wait_publishes_same_goal_without_game_control(self):
        ticks = iter((100, 101))
        with self.open(True) as owner:
            result = foreground.run(owner, self.authority, lambda a, g, b: sample(g, next(ticks)),
                lambda state: cli.reserve('wait', state, foreground.Limits()), foreground.Limits(),
                source_guard=lambda: cli.guard(owner, self.parent), sleeper=lambda _: None)
            self.assertEqual((result.samples, result.stop_reason), (2, 'terminal'))
            self.assertFalse(result.view()['game_time_advanced_by_wait'])
            self.assertEqual(owner.state.goal.digest, self.goal.digest)

    def test_wait_stops_on_stalled_ticks_and_does_not_retry_failed_reads(self):
        with self.open(True) as owner:
            result = foreground.run(owner, self.authority, lambda a, g, b: sample(g),
                lambda state: None, foreground.Limits(), sleeper=lambda _: None)
            self.assertEqual(result.stop_reason, 'game_tick_not_advanced')
            self.assertNotEqual(owner.state.progress.phase, 'satisfied')
        self.path = Path(self.directory.name) / 'failed-read'
        calls = 0
        def failed(*_):
            nonlocal calls
            calls += 1
            raise OSError('lost read')
        with self.open(True) as owner, self.assertRaises(OSError):
            foreground.run(owner, self.authority, failed, lambda state: None, foreground.Limits())
        self.assertEqual(calls, 1)

    def test_terminal_history_requires_original_parent_custody_and_cancel_is_monitor_only(self):
        with self.open(True) as owner:
            self.assertTrue(owner.cancel(lambda state: None))
            self.assertEqual(owner.state.progress.phase, 'cancelled')
            self.parent.valid = False
            with self.assertRaises(ValueError):
                cli.guard(owner, self.parent)
            self.parent.valid = True
            cli.guard(owner, self.parent)
            self.assertFalse(cli.packet('cancel', owner.state)['result']['placement_effects_discharged'])

    def test_full_maximum_output_and_final_publication_guard(self):
        goal = fixture(32)
        with self.open(True, goal=goal) as owner:
            self.accept(owner)
            cli.reserve('sample', owner.state)
            self.assertLessEqual(len(cli.output(cli.packet('sample', owner.state))), cli.MAX_OUTPUT)

    def test_cli_inspect_holds_both_owners_and_never_recontacts_native(self):
        with self.open(True):
            pass
        self.parent.address = ADDRESS
        raw = []
        def emit(value):
            self.parent.check()
            with self.assertRaises(OSError):
                self.open()
            raw.append(value)
            return True
        args = ['inspect', '--journal', str(self.path), '--batch-directory', self.goal.origin,
                '--batch-id', BATCH_ID, '--expected-goal', self.goal.digest]
        with patch.object(cli, 'open_batch', return_value=self.parent), patch.object(cli, 'write_output', emit), \
             patch.object(cli.native, 'Client') as client:
            self.assertEqual(cli.main(args), 0)
            client.assert_not_called()
        self.assertEqual(len(raw), 1)

    def test_cli_wrong_identity_overrides_and_short_output_do_not_sample(self):
        with self.open(True):
            pass
        base = ['inspect', '--journal', str(self.path), '--batch-directory', self.goal.origin,
                '--batch-id', BATCH_ID, '--expected-goal', self.goal.digest]
        for args in (base + ['--deadline-tick', '2000'], base[:-1] + ['f' * 64]):
            with patch.object(cli, 'open_batch', return_value=self.parent), patch.object(cli, 'write_output') as output, \
                 patch.object(cli.native, 'Client') as client:
                self.assertEqual(cli.main(args), 2)
                self.assertEqual(output.call_count, 1)
                client.assert_not_called()
        with patch.object(cli, 'open_batch', return_value=self.parent), patch.object(cli, 'write_output', return_value=False) as output:
            self.assertEqual(cli.main(base), 2)
            self.assertEqual(output.call_count, 1)


if __name__ == '__main__':
    unittest.main()
