"""Execute whole-plan POSIX publication, recovery, and retained-evidence tests.

These tests operate real private files and an explicit joined subprocess. They
provide no native DFHack, game, physical power-loss, or production evidence.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from build_placement_wire import Record, Rejected, canonical, field
import construction_monitor_store as single
from construction_monitor_rpc import Budget
import construction_plan as p
import construction_plan_store as s
from construction_receipt import Manifest
from test_construction_receipt import RECEIPT, TICK, building, item, operations, goal as single_goal, sample as single_sample

ROOT = Path(__file__).resolve().parents[1]
ADDRESS = ('127.0.0.1', 5000)
RENDER = lambda value: canonical(value.progress.view())


def goal(count=3, **policy):
    original = Record.decode(RECEIPT)
    receipts = []
    for index in range(count):
        kind = index % 3 + 1
        selection = replace(original.plan.before.selection, kind=kind,
                            item=42 + index, x=15 + index)
        before = replace(original.plan.before, selection=selection, sequence=index,
                         next_building=70 + index, next_job=90 + index, building_count=4 + index,
                         item=replace(original.plan.before.item, kind=kind, native_type=100 + kind))
        plan = replace(original.plan, key=f'plan-target-{index:02}', before=before)
        insertion = replace(original.insertion, kind=kind, building=70 + index,
                            job=90 + index, item=42 + index, pos=selection.target)
        receipts.append(replace(original, plan=plan, after=before.expected_after(), insertion=insertion).raw)
    return p.Goal(tuple(receipts), policy.pop('deadline', TICK + 100), **policy)


def sample(g, tick=TICK + 1, *, uninstalled=(), missing=()):
    buildings, items = [], []
    for child in g.goals:
        record = child.record
        insertion = record.insertion
        if insertion.building in missing:
            continue
        kind = ('', 'Bed', 'Chair', 'Table')[insertion.kind]
        x, y, z = insertion.pos
        buildings.append(building(insertion.building, kind, native_type=insertion.kind,
                                  bounds=(x, y, x, y, z)))
        items.append(item(insertion.item, kind.upper(), native_type=100 + insertion.kind,
                          holder=insertion.building, flags=0 if insertion.building in uninstalled else 256))
    count = len(g.receipts)
    capture = operations(tick, buildings=tuple(buildings), items=tuple(items),
                         horizons=(90 + count, 70 + count, 42 + count))
    manifest = Manifest(41, 'test-df', 'test-dfhack')
    return p.LinkedSample(manifest, g.receipts, replace(manifest, generation=987),
                          capture, manifest, g.receipts)


class PlanCustodyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-plan-custody-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.path = str(self.root / 'furnishing.plan')
        self.goal = goal()

    def owner(self, *, writable=True, create=False):
        return s.Journal(self.path, Budget(10000), writable=writable,
                         create=(self.goal, ADDRESS) if create else None)

    def saved(self, *, ready=False):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(self.goal), RENDER)
            if ready:
                journal.start_read()
                journal.accept(sample(self.goal, TICK + 2), RENDER)
        return Path(self.path).read_bytes()

    def test_maximum_selection_goal_and_all_rows_replay_as_one_terminal_result(self):
        self.goal = goal(32)
        self.assertGreater(len(self.goal.encode()), 8192)
        raw = self.saved(ready=True)
        state = s.replay(raw, Budget(10000))
        self.assertEqual((state.frames, state.progress.phase, state.progress.streak), (5, 'satisfied', 2))
        self.assertEqual(len(state.progress.assessments), 32)
        self.assertEqual(state.goal, self.goal)
        self.assertEqual(state.address, ADDRESS)
        with self.owner(writable=False) as journal, patch.object(single.os, 'fsync', side_effect=AssertionError('offline write')):
            self.assertEqual(journal.state, state)
            self.assertEqual(journal.view()['journal_head'], raw[-32:].hex())
            self.assertFalse(journal.cancel(RENDER))
            with self.assertRaises(Rejected):
                journal.start_read()
        self.assertEqual(Path(self.path).read_bytes(), raw)

    def test_single_profile_bytes_unchanged_and_formats_cannot_cross(self):
        raw = single.MAGIC
        previous = None
        transitions = [('goal', field(b'127.0.0.1:5000') + single_goal().encode()),
                       ('read_started', b''), ('sample', single_sample().encode())]
        for sequence, (kind, payload) in enumerate(transitions):
            encoded = single.frame(previous, kind, payload)
            previous = replace(single.transition(previous, kind, payload, Budget(10000)),
                               frames=sequence + 1, tail=encoded[-32:])
            raw += encoded
        self.assertEqual(hashlib.sha256(raw).hexdigest(),
                         '658de71f6935f20a5613d4ab08e5aa93d98ddcce40efbeedaa3fe4e66c3745bc')
        plan_raw = self.saved()
        for data, replay in ((raw, s.replay), (plan_raw, single.replay),
                             (s.MAGIC + raw[8:], s.replay),
                             (single.MAGIC + plan_raw[8:], single.replay)):
            with self.assertRaises(Rejected):
                replay(data, Budget(10000))
        with self.assertRaises(Rejected):
            single.Journal(self.path, Budget(10000))
        self.assertEqual(Path(self.path).read_bytes(), plan_raw)

    def test_truncated_frames_corruption_and_rehashed_impossible_transitions_refused(self):
        raw = self.saved()
        at, boundaries, complete = len(s.MAGIC), [], set()
        while at < len(raw):
            length, _, _ = s.HEADER.unpack(raw[at:at + s.HEADER.size])
            end = at + s.HEADER.size + length + 32
            boundaries.extend((at, at + 1, at + s.HEADER.size - 1,
                               at + s.HEADER.size, end - 33, end - 32, end - 1))
            complete.add(end)
            at = end
        for end in sorted(set(boundaries)):
            if end in complete:
                self.assertFalse(s.replay(raw[:end], Budget(10000)).progress.terminal)
                continue
            with self.subTest(prefix=end), self.assertRaises(Rejected):
                s.replay(raw[:end], Budget(10000))
        for at in sorted(set(boundaries + [0, 7, len(raw) // 2])):
            changed = raw[:at] + bytes([raw[at] ^ 128]) + raw[at + 1:]
            with self.subTest(corruption=at), self.assertRaises(Rejected):
                s.replay(changed, Budget(10000))
        with self.owner() as journal:
            for kind, payload in (('sample', sample(self.goal).encode()), ('goal', self.goal.encode()),
                                  ('read_started', b'extra'), ('cancel', b'extra')):
                with self.subTest(kind=kind), self.assertRaises(Rejected):
                    s.replay(raw + s.frame(journal.state, kind, payload), Budget(10000))
            journal.start_read()
            before = Path(self.path).read_bytes()
            substituted = replace(sample(self.goal, TICK + 2),
                                  after_records=(*self.goal.receipts[:-1], self.goal.receipts[0]))
            with self.assertRaises(Rejected):
                journal.accept(substituted, RENDER)
            self.assertTrue(journal.fenced)
            self.assertEqual(Path(self.path).read_bytes(), before)
        with self.owner() as journal:
            journal.cancel(RENDER)
        terminal = Path(self.path).read_bytes()
        with self.assertRaises(Rejected):
            s.replay(terminal + s.frame(journal.state, 'read_started', b''), Budget(10000))

    def test_reopened_read_has_no_publication_permit_and_resets_entire_streak(self):
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
            self.assertFalse(journal.state.progress.terminal)
            self.assertEqual(journal.state.progress.streak, 1)
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 3), RENDER)
            self.assertEqual(journal.state.progress.phase, 'satisfied')
            self.assertEqual(journal.state.goal.deadline, TICK + 100)

    def test_earlier_target_success_not_latched_across_restarts(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(self.goal, uninstalled=(72,)), RENDER)
            self.assertEqual(journal.state.progress.view()['condition_met_count'], 2)
        with self.owner() as journal:
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 2, uninstalled=(70,)), RENDER)
            self.assertEqual(journal.state.progress.phase, 'active')
            self.assertEqual(journal.state.progress.streak, 0)
            self.assertEqual(journal.state.progress.reason_building, 70)
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 3), RENDER)
            self.assertEqual(journal.state.progress.phase, 'candidate')
        with self.owner() as journal:
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 4), RENDER)
            self.assertEqual(journal.state.progress.phase, 'satisfied')

    def test_late_target_identity_loss_invalidates_complete_plan(self):
        self.saved()
        with self.owner() as journal:
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 2, missing=(72,)), RENDER)
            self.assertEqual((journal.state.progress.phase, journal.state.progress.reason_building), ('invalidated', 72))
            self.assertEqual(len(journal.state.progress.assessments), 3)
        with self.owner(writable=False) as journal:
            self.assertEqual(journal.state.progress.phase, 'invalidated')
            self.assertFalse(journal.cancel(RENDER))

    def test_restart_does_not_renew_observation_allowance_or_policy(self):
        self.goal = goal(max_observations=2, interval=3, stable_span=3)
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(self.goal, uninstalled=(72,)), RENDER)
        with self.owner() as journal:
            journal.start_read()
            journal.accept(sample(self.goal, TICK + 5, uninstalled=(72,)), RENDER)
            self.assertEqual(journal.state.progress.phase, 'expired')
            self.assertEqual(journal.state.progress.reason, 'sample_budget_exhausted')
        with self.owner(writable=False) as journal:
            self.assertEqual(journal.state.goal, self.goal)
            self.assertEqual(journal.state.progress.observations, 2)

    def test_plan_capacity_and_frames_reserved_before_read_intent(self):
        with self.owner(create=True) as journal:
            raw = Path(self.path).read_bytes()
            # Enough room for the old single-placement bound is insufficient for
            # all 32 before/after receipts around the maximum complete capture.
            old_bound = journal.length + single.MAX_BODY + 3 * (s.HEADER.size + 33)
            self.assertGreater(s.MAX_BODY, single.MAX_BODY)
            with patch.object(s, 'MAX_FILE', old_bound), self.assertRaises(Rejected):
                journal.start_read()
            with patch.object(s, 'MAX_FRAMES', journal.state.frames + 2), self.assertRaises(Rejected):
                journal.start_read()
            self.assertEqual(Path(self.path).read_bytes(), raw)
            self.assertFalse(journal.read_owned)
            journal.cancel(RENDER)
            self.assertEqual(journal.state.progress.phase, 'cancelled')

    def test_goal_and_read_intent_file_or_parent_sync_failure_never_mints_permission(self):
        for during_create in (True, False):
            for failure in (1, 2):
                self.path = str(self.root / f'failure-{during_create}-{failure}')
                actual, calls = os.fsync, 0
                def fsync(fd):
                    nonlocal calls
                    calls += 1
                    if calls == failure:
                        raise OSError('injected custody synchronization failure')
                    return actual(fd)
                if during_create:
                    with patch.object(single.os, 'fsync', side_effect=fsync), self.assertRaises(OSError):
                        self.owner(create=True)
                else:
                    with self.owner(create=True) as journal:
                        with patch.object(single.os, 'fsync', side_effect=fsync), self.assertRaises(OSError):
                            journal.start_read()
                        self.assertFalse(journal.read_owned)
                        self.assertTrue(journal.fenced)
                with self.owner(writable=False) as journal:
                    self.assertEqual(journal.state.progress.reading, not during_create)
                    self.assertFalse(journal.read_owned)
                    self.assertEqual(journal.state.progress.observations, 0)

    def test_failed_whole_result_reservation_retains_only_unknown_read(self):
        with self.owner(create=True) as journal:
            journal.start_read()
            raw = Path(self.path).read_bytes()
            def reject_result(candidate):
                self.assertEqual(len(candidate.progress.assessments), 3)
                raise Rejected('entire plan result exceeds output budget')
            with self.assertRaises(Rejected):
                journal.accept(sample(self.goal), reject_result)
            self.assertTrue(journal.fenced)
            self.assertFalse(journal.read_owned)
            self.assertEqual(Path(self.path).read_bytes(), raw)
        with self.owner(writable=False) as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertEqual(journal.state.progress.observations, 0)

    def test_terminal_sync_failure_replays_historical_sample_without_publication_permit(self):
        self.saved()
        with self.owner() as journal:
            journal.start_read()
            with patch.object(single.os, 'fsync', side_effect=OSError('injected sample sync loss')), self.assertRaises(OSError):
                journal.accept(sample(self.goal, TICK + 2), RENDER)
            self.assertFalse(journal.state.progress.terminal)
            self.assertTrue(journal.fenced)
        with self.owner(writable=False) as journal:
            self.assertEqual(journal.state.progress.phase, 'satisfied')
            self.assertFalse(journal.read_owned)
            self.assertFalse(journal.cancel(RENDER))

    def test_old_byte_and_path_substitution_fence_publication(self):
        for replace_path in (False, True):
            self.path = str(self.root / f'substitution-{replace_path}')
            with self.owner(create=True) as journal:
                journal.start_read()
                def substitute(candidate):
                    path = Path(self.path)
                    raw = bytearray(path.read_bytes())
                    if replace_path:
                        path.rename(self.root / 'old-owner')
                    else:
                        raw[100] ^= 1
                    path.write_bytes(raw)
                    path.chmod(0o600)
                with self.assertRaises(Rejected):
                    journal.accept(sample(self.goal), substitute)
                self.assertTrue(journal.fenced)
                self.assertFalse(journal.read_owned)
                self.assertEqual(journal.state.progress.observations, 0)

    def test_short_writes_complete_and_torn_sample_is_retained_without_repair(self):
        actual = os.write
        with patch.object(single.os, 'write', side_effect=lambda fd, raw: actual(fd, raw[:17])):
            self.saved()
        with self.owner() as journal:
            journal.start_read()
            before = Path(self.path).read_bytes()
            calls = 0
            def write(fd, raw):
                nonlocal calls
                calls += 1
                if calls == 1:
                    return actual(fd, raw[:23])
                raise OSError('injected partial plan sample')
            with patch.object(single.os, 'write', side_effect=write), self.assertRaises(OSError):
                journal.accept(sample(self.goal, TICK + 2), RENDER)
            self.assertTrue(journal.fenced)
        torn = Path(self.path).read_bytes()
        self.assertEqual(torn[:-23], before)
        with self.assertRaises(Rejected):
            self.owner()
        with self.assertRaises(FileExistsError):
            self.owner(create=True)
        self.assertEqual(Path(self.path).read_bytes(), torn)

    def test_real_subprocess_lock_then_offline_replay_without_writes(self):
        self.saved(ready=True)
        program = ('from construction_plan_store import Journal; from construction_monitor_rpc import Budget; '
                   'import sys; owner=Journal(sys.argv[1],Budget(10000)); '
                   'print(owner.state.progress.phase); owner.close()')
        command = [sys.executable, '-c', program, self.path]
        environment = dict(os.environ, PYTHONPATH=str(ROOT / 'scripts'), PYTHONDONTWRITEBYTECODE='1')
        with self.owner():
            blocked = subprocess.run(command, env=environment, capture_output=True, timeout=5)
            self.assertNotEqual(blocked.returncode, 0)
            self.assertIn(b'BlockingIOError', blocked.stderr)
        before = Path(self.path).read_bytes()
        completed = subprocess.run(command, env=environment, capture_output=True, timeout=5)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, b'satisfied\n')
        self.assertEqual(Path(self.path).read_bytes(), before)

    def test_noncanonical_file_custody_and_replaced_parent_refused(self):
        self.saved()
        for mode in (0o400, 0o640):
            Path(self.path).chmod(mode)
            with self.assertRaises(Rejected):
                self.owner()
        Path(self.path).chmod(0o600)
        link = self.root / 'hardlink'
        os.link(self.path, link)
        with self.assertRaises(Rejected):
            self.owner()
        link.unlink()
        link.symlink_to(self.path)
        with self.assertRaises((Rejected, OSError)):
            s.Journal(str(link), Budget(10000))
        directory = self.root / 'private'
        directory.mkdir(mode=0o700)
        self.path = str(directory / 'journal')
        with self.owner(create=True) as journal:
            directory.rename(self.root / 'renamed')
            directory.mkdir(mode=0o700)
            with self.assertRaises(Rejected):
                journal.check()
            self.assertTrue(journal.fenced)


if __name__ == '__main__':
    unittest.main(verbosity=2)
