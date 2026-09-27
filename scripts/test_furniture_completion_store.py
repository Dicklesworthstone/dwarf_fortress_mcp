"""Durable original-batch completion tests across POSIX custody and restart."""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import replace
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from build_placement_wire import Rejected, canonical, field
import construction_monitor_store as single_store
import construction_plan_store as plan_store
from construction_monitor_rpc import Budget
import furniture_batch as batch_module
import furniture_completion_store as store
from test_furniture_completion import ADDRESS, TICK, create_batch, origin_from, goal_from, sample

RENDER = lambda state: canonical(state.progress.view())


class FurnitureCompletionCustodyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-furnishing-completion-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.batch_path = self.root / 'batch'
        _, self.batch_id = create_batch(self.batch_path)
        self.origin = origin_from(self.batch_path, self.batch_id)
        self.goal = goal_from(self.origin)
        self.path = str(self.root / 'completion.journal')

    @contextmanager
    def owner(self, *, create=False, writable=True, bind=True):
        budget = Budget(60000)
        with store.Journal(self.path, budget, writable=writable,
                           create=(self.goal, ADDRESS) if create else None) as journal:
            if bind:
                with batch_module.Batch(str(self.batch_path), budget) as batch:
                    journal.bind_batch(batch)
                    yield journal
            else:
                yield journal

    def saved(self, *, ready=False):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(self.goal), RENDER)
            if ready:
                journal.start_read()
                journal.accept(sample(self.goal, TICK + 2), RENDER)
        return Path(self.path).read_bytes()

    def fresh_case(self, name):
        self.batch_path = self.root / ('batch-' + name)
        _, self.batch_id = create_batch(self.batch_path)
        self.origin = origin_from(self.batch_path, self.batch_id)
        self.goal = goal_from(self.origin)
        self.path = str(self.root / (name + '.journal'))

    def replace_original_inode(self, kind):
        path = (self.batch_path / 'effects' / self.origin.children[-1].name if kind == 'child'
                else self.batch_path / 'steps.jsonl')
        replacement = self.root / ('replacement-' + kind)
        replacement.write_bytes(path.read_bytes())
        replacement.chmod(0o600)
        old = path.stat().st_ino
        os.replace(replacement, path)
        self.assertNotEqual(path.stat().st_ino, old)

    def test_complete_goal_and_every_observation_replay_to_the_same_terminal_state(self):
        raw = self.saved(ready=True)
        state = store.replay(raw, Budget(60000))
        self.assertEqual((state.frames, state.progress.phase, state.progress.streak), (5, 'satisfied', 2))
        self.assertEqual(state.goal, self.goal)
        self.assertEqual(state.goal.origin, self.origin)
        self.assertEqual(state.address, ADDRESS)
        self.assertEqual(state.progress.goal_digest, self.goal.digest)
        self.assertEqual(len(state.progress.assessments), 3)
        with self.owner(writable=False, bind=False) as journal, patch('os.fsync', side_effect=AssertionError('offline write')):
            self.assertEqual(journal.state, state)
            self.assertEqual(journal.view()['journal_head'], raw[-32:].hex())
            self.assertFalse(journal.cancel(RENDER))
            self.assertFalse(journal.read_owned)
        self.assertEqual(Path(self.path).read_bytes(), raw)

    def test_native_read_needs_original_batch_binding_and_the_same_budget(self):
        with self.owner(create=True, bind=False) as journal:
            before = Path(self.path).read_bytes()
            with self.assertRaises(Rejected):
                journal.start_read()
            with self.assertRaises(Rejected):
                journal.accept(sample(self.goal), RENDER)
            with batch_module.Batch(str(self.batch_path), Budget(60000)) as wrong_budget:
                with self.assertRaises(Rejected):
                    journal.bind_batch(wrong_budget)
            self.assertEqual(Path(self.path).read_bytes(), before)
            self.assertFalse(journal.read_owned)
            with batch_module.Batch(str(self.batch_path), journal.budget) as original:
                journal.bind_batch(original)
                journal.start_read()
                journal.accept(sample(self.goal), RENDER)
                self.assertEqual(journal.state.progress.streak, 1)

    def test_wrong_batch_and_nonoriginal_endpoint_fail_before_read_intent(self):
        other_path = self.root / 'other-batch'
        create_batch(other_path)
        with self.owner(create=True, bind=False) as journal:
            before = Path(self.path).read_bytes()
            with batch_module.Batch(str(other_path), journal.budget) as other:
                with self.assertRaises(Rejected):
                    journal.bind_batch(other)
            self.assertTrue(journal.fenced)
            self.assertFalse(journal.read_owned)
            self.assertEqual(Path(self.path).read_bytes(), before)
        with self.assertRaises(Rejected):
            store.Journal(str(self.root / 'wrong-endpoint'), Budget(60000), writable=True,
                          create=(self.goal, ('127.0.0.1', 5001)))
        self.assertFalse((self.root / 'wrong-endpoint').exists())

    def test_offline_cancel_remains_available_after_original_batch_is_unavailable(self):
        self.saved()
        raw = Path(self.path).read_bytes()
        self.batch_path.rename(self.root / 'unavailable-original')
        with self.owner(bind=False) as journal:
            self.assertEqual(journal.state.progress.streak, 1)
            with self.assertRaises(Rejected):
                journal.start_read()
            self.assertTrue(journal.cancel(RENDER))
            self.assertEqual(journal.state.progress.phase, 'cancelled')
            self.assertFalse(journal.state.progress.view()['placement_effect_discharged'])
        self.assertTrue(Path(self.path).read_bytes().startswith(raw))
        with self.owner(bind=False, writable=False) as journal:
            self.assertFalse(journal.cancel(RENDER))

    def test_distinct_journal_profile_and_domain_refuse_plan_or_single_monitor_bytes(self):
        raw = self.saved()
        payload = field(b'127.0.0.1:5000') + self.goal.condition.encode()
        old = plan_store.MAGIC + plan_store.frame(None, 'goal', payload)
        for data, replay in ((old, store.replay), (raw, plan_store.replay), (raw, single_store.replay),
                             (store.MAGIC + old[8:], store.replay),
                             (plan_store.MAGIC + raw[8:], plan_store.replay)):
            with self.assertRaises(Rejected):
                replay(data, Budget(60000))
        for owner in (single_store.Journal, plan_store.Journal):
            with self.assertRaises(Rejected):
                owner(self.path, Budget(60000))
        self.assertEqual(Path(self.path).read_bytes(), raw)

    def test_torn_corrupt_and_rehashed_impossible_transitions_are_refused_without_repair(self):
        raw = self.saved()
        offset, complete, cuts = len(store.MAGIC), set(), []
        while offset < len(raw):
            length, _, _ = store.HEADER.unpack(raw[offset:offset + store.HEADER.size])
            end = offset + store.HEADER.size + length + 32
            cuts.extend((offset, offset + 1, offset + store.HEADER.size,
                         end - 33, end - 32, end - 1))
            complete.add(end)
            offset = end
        for end in sorted(set(cuts) - complete):
            with self.subTest(truncated=end), self.assertRaises(Rejected):
                store.replay(raw[:end], Budget(60000))
        for index in (0, 7, 9, len(raw) // 2, len(raw) - 1):
            with self.subTest(corruption=index), self.assertRaises(Rejected):
                store.replay(raw[:index] + bytes([raw[index] ^ 128]) + raw[index + 1:], Budget(60000))
        state = store.replay(raw, Budget(60000))
        for kind, payload in (('sample', sample(self.goal).encode()), ('read_started', b'extra'),
                              ('cancel', b'extra'), ('goal', field(b'127.0.0.1:5000') + self.goal.encode())):
            with self.subTest(transition=kind), self.assertRaises(Rejected):
                store.replay(raw + store.frame(state, kind, payload), Budget(60000))
        path = Path(self.path)
        path.write_bytes(raw[:-1])
        with self.assertRaises(Rejected):
            with self.owner(bind=False):
                pass
        with self.assertRaises(FileExistsError):
            with self.owner(create=True, bind=False):
                pass
        self.assertEqual(path.read_bytes(), raw[:-1])

    def test_reopened_unknown_read_has_no_permission_and_resets_whole_plan_stability(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(self.goal), RENDER)
            journal.start_read()
        before = Path(self.path).read_bytes()
        with self.owner() as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertFalse(journal.read_owned)
            with self.assertRaises(Rejected):
                journal.accept(sample(self.goal, TICK + 2), RENDER)
            self.assertEqual(Path(self.path).read_bytes(), before)
            journal.start_read()
            self.assertEqual((journal.state.progress.streak, journal.state.progress.interruptions), (0, 1))
            journal.accept(sample(self.goal, TICK + 2), RENDER)
            self.assertEqual(journal.state.progress.phase, 'candidate')
            self.assertEqual(journal.state.goal, self.goal)
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 3), RENDER)
            self.assertEqual(journal.state.progress.phase, 'satisfied')

    def test_failed_complete_result_reservation_retains_only_unknown_read(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            before = Path(self.path).read_bytes()
            def refuse(candidate):
                self.assertEqual(candidate.progress.goal_digest, self.goal.digest)
                self.assertEqual(len(candidate.progress.assessments), 3)
                raise Rejected('complete original batch result does not fit')
            with self.assertRaises(Rejected):
                journal.accept(sample(self.goal), refuse)
            self.assertTrue(journal.fenced)
            self.assertFalse(journal.read_owned)
            self.assertEqual(Path(self.path).read_bytes(), before)
        with self.owner(bind=False, writable=False) as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 0)

    def test_original_child_or_index_substitution_during_render_fences_before_sample_write(self):
        for kind in ('child', 'index'):
            with self.subTest(original=kind):
                self.fresh_case(kind)
                with self.owner(create=True) as journal:
                    journal.start_read()
                    before = Path(self.path).read_bytes()
                    def substitute(candidate):
                        self.assertEqual(len(candidate.progress.assessments), 3)
                        self.replace_original_inode(kind)
                    with self.assertRaises(Rejected):
                        journal.accept(sample(self.goal), substitute)
                    self.assertTrue(journal.fenced)
                    self.assertFalse(journal.read_owned)
                    self.assertEqual(journal.state.progress.observations, 0)
                    self.assertEqual(Path(self.path).read_bytes(), before)

    def test_original_child_or_index_substitution_after_parent_sync_prevents_acknowledgment(self):
        for kind in ('child', 'index'):
            with self.subTest(original=kind):
                self.fresh_case(kind)
                self.saved()
                with self.owner() as journal:
                    journal.start_read()
                    old = journal.state
                    actual, calls = os.fsync, 0
                    def substitute_after_sync(fd):
                        nonlocal calls
                        actual(fd)
                        calls += 1
                        if calls == 2:
                            self.replace_original_inode(kind)
                    with patch('os.fsync', side_effect=substitute_after_sync), self.assertRaises(Rejected):
                        journal.accept(sample(self.goal, TICK + 2), RENDER)
                    self.assertEqual(calls, 2)
                    self.assertTrue(journal.fenced)
                    self.assertFalse(journal.read_owned)
                    self.assertEqual(journal.state, old)
                    self.assertFalse(journal.state.progress.terminal)
                # Complete bytes remain historical evidence; they cannot acknowledge
                # the failed call or reacquire replaced original custody.
                with self.owner(bind=False, writable=False) as journal:
                    self.assertEqual(journal.state.progress.phase, 'satisfied')
                    self.assertFalse(journal.read_owned)
                with self.assertRaises(Rejected):
                    with self.owner():
                        pass

    def test_read_intent_and_terminal_sync_failures_never_mint_publication_permission(self):
        for stage in ('read_started', 'sample'):
            for point in (1, 2):
                self.path = str(self.root / f'{stage}-sync-{point}')
                with self.owner(create=True) as journal:
                    if stage == 'sample':
                        journal.start_read()
                    actual, calls = os.fsync, 0
                    def fail(fd):
                        nonlocal calls
                        calls += 1
                        if calls == point:
                            raise OSError('injected completion synchronization failure')
                        actual(fd)
                    with patch('os.fsync', side_effect=fail), self.assertRaises(OSError):
                        if stage == 'read_started':
                            journal.start_read()
                        else:
                            journal.accept(sample(self.goal), RENDER)
                    self.assertTrue(journal.fenced)
                    self.assertFalse(journal.read_owned)
                    self.assertEqual(journal.state.progress.observations, 0)
                with self.owner(bind=False, writable=False) as journal:
                    self.assertFalse(journal.read_owned)
                    self.assertEqual(journal.state.progress.observations, int(stage == 'sample'))

    def test_source_and_sample_replay_checks_are_not_bypassed_by_valid_frame_hashes(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            raw, state = Path(self.path).read_bytes(), journal.state
            good = sample(self.goal)
            changed_source = replace(good.before, df_version='different-df')
            bad = replace(good, before=changed_source, after=changed_source,
                          operations=replace(changed_source, generation=good.operations.generation))
            for changed in (bad, replace(good, after_records=(*good.after_records[:-1], good.after_records[0]))):
                forged = raw + store.frame(state, 'sample', changed.encode())
                with self.assertRaises(Rejected):
                    store.replay(forged, Budget(60000))
            self.assertEqual(Path(self.path).read_bytes(), raw)

    def test_complete_sample_and_cancel_capacity_are_reserved_before_read_intent(self):
        with self.owner(create=True) as journal:
            before = Path(self.path).read_bytes()
            with patch.object(store, 'MAX_FILE', journal.length + store.MAX_BODY), self.assertRaises(Rejected):
                journal.start_read()
            with patch.object(store, 'MAX_FRAMES', journal.state.frames + 2), self.assertRaises(Rejected):
                journal.start_read()
            self.assertEqual(Path(self.path).read_bytes(), before)
            self.assertFalse(journal.read_owned)
            self.assertTrue(journal.cancel(RENDER))
            self.assertEqual(journal.state.progress.phase, 'cancelled')

    def test_noncanonical_monitor_custody_and_byte_identical_path_replacement_are_refused(self):
        self.saved()
        for mode in (0o400, 0o640):
            Path(self.path).chmod(mode)
            with self.assertRaises(Rejected):
                with self.owner(bind=False):
                    pass
        Path(self.path).chmod(0o600)
        link = self.root / 'extra-link'
        os.link(self.path, link)
        with self.assertRaises(Rejected):
            with self.owner(bind=False):
                pass
        link.unlink()
        with self.owner() as journal:
            journal.start_read()
            def substitute(candidate):
                path = Path(self.path)
                original = path.read_bytes()
                path.rename(self.root / 'old-monitor')
                path.write_bytes(original)
                path.chmod(0o600)
            with self.assertRaises(Rejected):
                journal.accept(sample(self.goal, TICK + 2), substitute)
            self.assertTrue(journal.fenced)
            self.assertFalse(journal.read_owned)


if __name__ == '__main__':
    unittest.main(verbosity=2)
