"""Joint room journal tests using real original custody and raw native codecs.

The local placement receipts and map/operations observations are fixtures, not
native DFHack execution or an acquisition attestation.
"""
from contextlib import contextmanager
from dataclasses import replace
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_placement_rpc as placement_rpc
import build_placement_wire as wire
import construction_monitor_store as custody
import construction_plan as condition
import construction_plan_store as selected_store
import furniture_batch as batch_module
import furniture_completion as completion
import furniture_completion_store as furnishing_store
import room_readiness_store as store
import room_readiness_fixtures as fixture
from room_readiness_rpc import Budget

ADDRESS = ('127.0.0.1', 5000)
SOURCE = placement_rpc.Manifest(fixture.BUILD.generation, *fixture.BUILD.software)


def create_room_batch(path: Path, *, large=False, address=ADDRESS) -> completion.Origin:
    """Create complete real batch/3 files through their existing private owners."""
    path.mkdir(mode=0o700)
    (path / 'effects').mkdir(mode=0o700)
    handoff = fixture.handoff(fixture.room(large), address=f'{address[0]}:{address[1]}')
    plan = handoff.allocation.plan()
    manifest = {'schema': batch_module.ROOM_SCHEMA, 'nonce': 'ab' * 24,
        'plan': plan.json(), 'room_handoff': handoff.json(), 'source': SOURCE.view(),
        'endpoint': f'{address[0]}:{address[1]}', 'folder': 'region1', 'site': 2,
        'dimensions': list(fixture.DIMENSIONS), 'first_tick': 200,
        'root_identity': [path.stat().st_dev, path.stat().st_ino],
        'effects_identity': [(path / 'effects').stat().st_dev, (path / 'effects').stat().st_ino]}
    budget = Budget(60000)
    with batch_module.root_lock(str(path), budget) as root:
        for name, raw in (('batch.json', batch_module.seal(manifest)),
                          ('steps.jsonl', batch_module.HEADER)):
            file = batch_module.File(root, name, budget, True, raw,
                                     maximum=(batch_module.MAX_ROOM_DEFINITION if name == 'batch.json'
                                              else batch_module.MAX_FILE))
            file.close()
    records = tuple(wire.Record.decode(raw) for raw in fixture.receipts(handoff))
    with batch_module.Batch(str(path), budget, True) as batch:
        for index, (step, record) in enumerate(zip(plan.ordered, records)):
            native = wire.Plan(batch.key(step), record.plan.before)
            journal = batch.effects.create(native, SOURCE, address)
            batch.register(step)
            prepared = wire.Record(native, 'prepared', 'none')
            journal.retain(placement_rpc.Reply(SOURCE, False, index + 1, record=prepared), prepared=True)
            journal.append('dispatch', {'plan_digest': native.digest.hex()})
            placed = wire.Record(native, 'placed', 'none', native.before.expected_after(), record.insertion)
            journal.retain(placement_rpc.Reply(SOURCE, False, index + 1, record=placed))
        batch.audit()
        return completion.Origin.from_batch(batch, batch.id)


def goal_from(origin: completion.Origin, **policy) -> store.Goal:
    return store.Goal(origin, condition.Goal(origin.receipts, policy.pop('deadline', 10000),
                                            stable_span=policy.pop('stable_span', 10), **policy))


def sample(goal: store.Goal, tick=300, **changes):
    return fixture.sample(goal.readiness_goal, tick, **changes)


def render(state):
    return wire.canonical(state.progress.view())


class RoomReadinessStoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-joint-room-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.batch_path = self.root / 'batch'
        self.origin = create_room_batch(self.batch_path)
        self.goal = goal_from(self.origin)
        self.path = self.root / 'readiness.journal'

    @contextmanager
    def owner(self, *, create=False, writable=True, bind=True):
        budget = Budget(60000)
        with store.Journal(str(self.path), budget, writable=writable,
                           create=(self.goal, ADDRESS) if create else None) as journal:
            if bind:
                with batch_module.Batch(str(self.batch_path), budget) as batch:
                    journal.bind_batch(batch)
                    yield journal
            else:
                yield journal

    def saved(self, *, satisfied=False):
        with self.owner(create=True) as journal:
            journal.start_read()
            journal.accept(sample(self.goal), render)
            if satisfied:
                journal.start_read()
                journal.accept(sample(self.goal, 310), render)
        return self.path.read_bytes()

    def replace_original(self, kind):
        path = (self.batch_path / 'effects' / self.origin.children[-1].name if kind == 'child'
                else self.batch_path / 'steps.jsonl')
        replacement = self.root / ('replacement-' + kind)
        replacement.write_bytes(path.read_bytes())
        replacement.chmod(0o600)
        os.replace(replacement, path)

    def test_original_goal_roundtrip_keeps_room_policy_receipts_and_distinct_identity(self):
        restored = store.Goal.decode(self.goal.encode())
        self.assertEqual(restored, self.goal)
        self.assertEqual(restored.readiness_goal.room, self.origin.room_handoff)
        self.assertEqual(restored.condition.receipts, self.goal.receipts)
        self.assertEqual(restored.deadline, 10000)
        self.assertNotEqual(restored.digest, restored.readiness_goal.digest)
        for name in ('deadline', 'interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'):
            value = getattr(self.goal.condition, name) + (-1 if name == 'max_observations' else 1)
            changed = store.Goal(self.origin, replace(self.goal.condition, **{name: value}))
            self.assertNotEqual(changed.digest, self.goal.digest)
        smaller = replace(self.goal.condition, receipts=self.goal.receipts[:-1])
        self.assertRaises(ValueError, store.Goal, self.origin, smaller)
        for raw in (self.goal.encode()[:-1], self.goal.encode() + b'x', b'DFMFCG03' + self.goal.encode()[8:]):
            self.assertRaises(ValueError, store.Goal.decode, raw)
        def stop():
            raise ValueError('bounded decode interrupted')
        self.assertRaises(ValueError, store.Goal.decode, self.goal.encode(), stop)

    def test_plain_furnishing_batch_cannot_substitute_for_original_room(self):
        from test_furniture_completion import create_batch, origin_from
        _, identity = create_batch(self.root / 'plain')
        origin = origin_from(self.root / 'plain', identity)
        self.assertRaises(ValueError, store.Goal, origin, condition.Goal(origin.receipts, 1000000))

    def test_full_raw_history_replays_joint_streak_and_keeps_original_bytes_unchanged(self):
        original_files = {path.relative_to(self.batch_path): path.read_bytes()
                          for path in self.batch_path.rglob('*') if path.is_file()}
        raw = self.saved(satisfied=True)
        state = store.replay(raw, Budget(60000))
        self.assertEqual((state.frames, state.progress.phase, state.progress.streak), (5, 'satisfied', 2))
        self.assertEqual(state.goal, self.goal)
        self.assertEqual(state.progress.goal_digest, self.goal.digest)
        self.assertTrue(state.progress.view()['room_readiness_sampled_condition'])
        self.assertFalse(state.progress.view()['room_completion_proven'])
        self.assertFalse(state.progress.view()['atomic_cross_profile_snapshot_proven'])
        with self.owner(writable=False, bind=False) as journal, patch('os.fsync', side_effect=AssertionError('write')):
            self.assertEqual(journal.state, state)
            self.assertFalse(journal.cancel(render))
            self.assertFalse(journal.read_owned)
        self.assertEqual(self.path.read_bytes(), raw)
        self.assertEqual(original_files, {path.relative_to(self.batch_path): path.read_bytes()
                         for path in self.batch_path.rglob('*') if path.is_file()})

    def test_read_and_publication_require_original_batch_and_shared_budget(self):
        with self.owner(create=True, bind=False) as journal:
            before = self.path.read_bytes()
            self.assertRaises(ValueError, journal.start_read)
            self.assertRaises(ValueError, journal.accept, sample(self.goal), render)
            with batch_module.Batch(str(self.batch_path), Budget(60000)) as other_budget:
                self.assertRaises(ValueError, journal.bind_batch, other_budget)
            self.assertEqual(self.path.read_bytes(), before)
            with batch_module.Batch(str(self.batch_path), journal.budget) as original:
                journal.bind_batch(original)
                journal.start_read()
                journal.accept(sample(self.goal), render)
                self.assertEqual(journal.state.progress.streak, 1)
        wrong = self.root / 'wrong-endpoint'
        self.assertRaises(ValueError, store.Journal, str(wrong), Budget(60000), writable=True,
                          create=(self.goal, ('127.0.0.1', 5001)))
        self.assertFalse(wrong.exists())

    def test_clean_restart_keeps_samples_but_unfinished_read_cannot_publish_or_keep_streak(self):
        self.saved()
        with self.owner() as journal:
            self.assertEqual(journal.state.progress.streak, 1)
            journal.start_read()
        raw = self.path.read_bytes()
        with self.owner() as journal:
            self.assertTrue(journal.state.progress.reading)
            self.assertFalse(journal.read_owned)
            self.assertRaises(ValueError, journal.accept, sample(self.goal, 310), render)
            self.assertEqual(self.path.read_bytes(), raw)
            journal.start_read()
            self.assertEqual((journal.state.progress.streak, journal.state.progress.interruptions), (0, 1))
            journal.accept(sample(self.goal, 310), render)
            self.assertEqual(journal.state.progress.streak, 1)
            self.assertFalse(journal.state.progress.terminal)
            journal.start_read()
            journal.accept(sample(self.goal, 320), render)
            self.assertEqual(journal.state.progress.phase, 'satisfied')
            self.assertEqual(journal.state.goal.deadline, self.goal.deadline)

    def test_clean_restart_cannot_combine_lost_walls_separate_success_or_old_gaps(self):
        self.saved()
        wall = min(fixture.room_terrain.selection(self.goal.origin.room_handoff.room_plan).walls)
        with self.owner() as journal:
            for tick, changes in ((310, {'overrides': {wall: fixture.tile(3)}}),
                                  (320, {'pending_item': self.goal.goals[0].record.insertion.item}),
                                  (330, {}), (2000, {})):
                journal.start_read()
                journal.accept(sample(self.goal, tick, **changes), render)
                self.assertFalse(journal.state.progress.terminal)
            self.assertEqual(journal.state.progress.streak, 1)
            self.assertEqual(journal.state.progress.first_tick, 2000)
            journal.start_read()
            journal.accept(sample(self.goal, 2010), render)
            self.assertEqual(journal.state.progress.phase, 'satisfied')

    def test_map_epoch_change_is_terminal_and_original_placement_software_is_bound(self):
        self.saved()
        with self.owner() as journal:
            journal.start_read()
            journal.accept(sample(self.goal, 310, map_manifest=replace(fixture.MAP, generation=30)), render)
            self.assertEqual(journal.state.progress.phase, 'invalidated')
            self.assertEqual(journal.state.progress.reason, 'room_map_source_changed')
        self.path = self.root / 'source.journal'
        with self.owner(create=True) as journal:
            journal.start_read()
            good = sample(self.goal)
            changed = replace(good.furnishings.before, df_version='changed')
            bad = replace(good, furnishings=replace(good.furnishings, before=changed, after=changed))
            self.assertRaises(ValueError, journal.accept, bad, render)
            self.assertTrue(journal.fenced)
            self.assertEqual(journal.state.progress.observations, 0)

    def test_offline_cancel_survives_original_loss_without_read_authority(self):
        before = self.saved()
        self.batch_path.rename(self.root / 'original-unavailable')
        with self.owner(bind=False) as journal:
            self.assertRaises(ValueError, journal.start_read)
            self.assertTrue(journal.cancel(render))
            self.assertEqual(journal.state.progress.phase, 'cancelled')
            self.assertEqual(journal.state.goal.origin.room_handoff, self.origin.room_handoff)
            self.assertFalse(journal.state.progress.view()['placement_effect_discharged'])
        self.assertTrue(self.path.read_bytes().startswith(before))
        with self.owner(bind=False, writable=False) as journal:
            self.assertFalse(journal.cancel(render))

    def test_legacy_journal_profiles_cannot_enter_joint_replay(self):
        raw = self.saved()
        payload = wire.field(b'127.0.0.1:5000') + self.goal.condition.encode()
        old = selected_store.MAGIC + selected_store.frame(None, 'goal', payload)
        for bytes_value, decoder in ((old, store.replay), (store.MAGIC + old[8:], store.replay),
                                     (raw, selected_store.replay), (raw, furnishing_store.replay),
                                     (raw, custody.replay)):
            self.assertRaises(ValueError, decoder, bytes_value, Budget(60000))

    def test_torn_corrupt_rehashed_or_short_reads_refuse_without_repair(self):
        raw = self.saved()
        offset, ends, cuts = len(store.MAGIC), set(), []
        while offset < len(raw):
            length, _, _ = store.HEADER.unpack(raw[offset:offset + store.HEADER.size])
            end = offset + store.HEADER.size + length + 32
            cuts.extend((offset, offset + 1, offset + store.HEADER.size, end - 32, end - 1))
            ends.add(end)
            offset = end
        for end in sorted(set(cuts) - ends):
            self.assertRaises(ValueError, store.replay, raw[:end], Budget(60000))
        for index in (0, 7, 10, len(raw) // 2, len(raw) - 1):
            changed = raw[:index] + bytes([raw[index] ^ 128]) + raw[index + 1:]
            self.assertRaises(ValueError, store.replay, changed, Budget(60000))
        state = store.replay(raw, Budget(60000))
        for kind, payload in (('sample', sample(self.goal).encode()), ('read_started', b'x'), ('cancel', b'x')):
            self.assertRaises(ValueError, store.replay, raw + store.frame(state, kind, payload), Budget(60000))
        def short(offset, count):
            return raw[offset:offset + count - 1] if offset else raw[:count]
        self.assertRaises(ValueError, store.replay_reader, short, len(raw), Budget(60000))
        self.path.write_bytes(raw[:-1])
        self.assertRaises(ValueError, store.Journal, str(self.path), Budget(60000))
        self.assertEqual(self.path.read_bytes(), raw[:-1])

    def test_failed_output_or_original_change_during_render_keeps_only_unknown_read(self):
        for name in ('output', 'child', 'index'):
            with self.subTest(boundary=name):
                if name != 'output':
                    self.batch_path = self.root / ('batch-' + name)
                    self.origin = create_room_batch(self.batch_path)
                    self.goal = goal_from(self.origin)
                self.path = self.root / (name + '.journal')
                with self.owner(create=True) as journal:
                    journal.start_read()
                    before = self.path.read_bytes()
                    def refuse(candidate):
                        self.assertEqual(candidate.progress.goal_digest, self.goal.digest)
                        if name == 'output':
                            raise ValueError('whole joint result does not fit')
                        self.replace_original(name)
                    self.assertRaises(ValueError, journal.accept, sample(self.goal), refuse)
                    self.assertTrue(journal.fenced)
                    self.assertFalse(journal.read_owned)
                    self.assertEqual(self.path.read_bytes(), before)
                with self.owner(bind=False, writable=False) as journal:
                    self.assertTrue(journal.state.progress.reading)
                    self.assertEqual(journal.state.progress.observations, 0)

    def test_original_replacement_after_fsync_prevents_acknowledging_complete_bytes(self):
        self.saved()
        with self.owner() as journal:
            journal.start_read()
            old = journal.state
            actual, calls = os.fsync, 0
            def replace_after_sync(fd):
                nonlocal calls
                actual(fd)
                calls += 1
                if calls == 2:
                    self.replace_original('child')
            with patch('os.fsync', side_effect=replace_after_sync):
                self.assertRaises(ValueError, journal.accept, sample(self.goal, 310), render)
            self.assertEqual(calls, 2)
            self.assertTrue(journal.fenced)
            self.assertFalse(journal.read_owned)
            self.assertEqual(journal.state, old)
        with self.owner(bind=False, writable=False) as journal:
            self.assertEqual(journal.state.progress.phase, 'satisfied')
            self.assertFalse(journal.read_owned)

    def test_sync_failure_and_capacity_never_restore_publication_permission(self):
        for stage in ('read_started', 'sample'):
            self.path = self.root / (stage + '.journal')
            with self.owner(create=True) as journal:
                if stage == 'sample':
                    journal.start_read()
                with patch('os.fsync', side_effect=OSError('injected fsync failure')):
                    if stage == 'sample':
                        self.assertRaises(OSError, journal.accept, sample(self.goal), render)
                    else:
                        self.assertRaises(OSError, journal.start_read)
                self.assertTrue(journal.fenced)
                self.assertFalse(journal.read_owned)
            with self.owner(bind=False, writable=False) as journal:
                self.assertFalse(journal.read_owned)
        self.path = self.root / 'capacity.journal'
        with self.owner(create=True) as journal:
            before = self.path.read_bytes()
            with patch.object(store, 'MAX_FILE', journal.length + store.MAX_BODY):
                self.assertRaises(ValueError, journal.start_read)
            with patch.object(store, 'MAX_FRAMES', journal.state.frames + 2):
                self.assertRaises(ValueError, journal.start_read)
            self.assertEqual(self.path.read_bytes(), before)
            self.assertTrue(journal.cancel(render))

    def test_noncanonical_custody_and_path_replacement_are_refused(self):
        self.saved()
        for mode in (0o400, 0o640):
            self.path.chmod(mode)
            self.assertRaises(ValueError, store.Journal, str(self.path), Budget(60000))
        self.path.chmod(0o600)
        linked = self.root / 'extra-link'
        os.link(self.path, linked)
        self.assertRaises(ValueError, store.Journal, str(self.path), Budget(60000))
        linked.unlink()
        symlink = self.root / 'symlink'
        symlink.symlink_to(self.path)
        self.assertRaises((OSError, ValueError), store.Journal, str(symlink), Budget(60000))
        with self.owner() as journal:
            journal.start_read()
            def replace_path(candidate):
                raw = self.path.read_bytes()
                self.path.rename(self.root / 'old-journal')
                self.path.write_bytes(raw)
                self.path.chmod(0o600)
            self.assertRaises(ValueError, journal.accept, sample(self.goal, 310), replace_path)
            self.assertTrue(journal.fenced)

    def test_32_targets_and_all_exclusions_survive_durable_joint_completion(self):
        self.batch_path = self.root / 'maximum-batch'
        self.origin = create_room_batch(self.batch_path, large=True)
        self.goal = goal_from(self.origin)
        raw = self.saved(satisfied=True)
        restored = store.replay(raw, Budget(60000))
        self.assertEqual(len(restored.goal.receipts), 32)
        self.assertEqual(len(restored.goal.origin.room_handoff.allocation.request.excluded_items), 646)
        self.assertEqual(restored.progress.phase, 'satisfied')
        self.assertEqual(len(restored.progress.assessments), 32)
        self.assertEqual(restored.goal.origin.encode(), self.origin.encode())


if __name__ == '__main__':
    unittest.main()
