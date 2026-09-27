"""Exact batch-to-completion provenance and whole-plan condition regressions.

Fixtures use the real private batch/placement custody owners and independent
native receipt vectors. They do not contact DFHack or authorize game effects.
"""
from __future__ import annotations

from dataclasses import replace
import json
import os
from pathlib import Path
import tempfile
import unittest

import build_placement_rpc as placement_rpc
import build_placement_store as placement_store
import build_placement_wire as wire
import construction_plan as plan
import construction_receipt as receipt
import furniture_batch as batch_module
import furniture_completion as completion
import furniture_plan as furniture
from test_construction_receipt import RECEIPT, TICK
from test_construction_plan_store import sample as plan_sample

ADDRESS = ('127.0.0.1', 5000)
SOURCE = placement_rpc.Manifest(41, 'test-df', 'test-dfhack')
GUARD = lambda: None


def create_batch(path: Path, count: int = 3, *, placed: int | None = None,
                 registered: int | None = None) -> tuple[furniture.FurniturePlan, str]:
    """Create authentic local custody with deterministic, fully replayable receipts."""
    path.mkdir(mode=0o700)
    (path / 'effects').mkdir(mode=0o700)
    steps = tuple(furniture.Step(f'r{count - index - 1:02}', furniture.KINDS[index % 3],
                  42 + index, (15 + 3 * index, 15, 2),
                  (f'r{count - index:02}',) if index else ()) for index in range(count))
    selected = furniture.FurniturePlan(steps)
    placed = count if placed is None else placed
    registered = placed if registered is None else registered
    value = {'schema': batch_module.SCHEMA, 'nonce': 'ab' * 24, 'plan': selected.json(),
             'source': SOURCE.view(), 'endpoint': f'{ADDRESS[0]}:{ADDRESS[1]}',
             'folder': 'region1', 'site': 2, 'dimensions': [256, 64, 8],
             'first_tick': TICK,
             'root_identity': [path.stat().st_dev, path.stat().st_ino],
             'effects_identity': [(path / 'effects').stat().st_dev, (path / 'effects').stat().st_ino]}
    budget = placement_rpc.Budget(60000)
    with batch_module.root_lock(str(path), budget) as root:
        for name, raw in (('batch.json', batch_module.seal(value)),
                          ('steps.jsonl', batch_module.HEADER)):
            file = batch_module.File(root, name, budget, True, raw)
            file.close()
    original = wire.Record.decode(RECEIPT)
    with batch_module.Batch(str(path), budget, True) as batch:
        for index, step in enumerate(selected.ordered[:placed]):
            before = replace(original.plan.before, selection=batch_module.selection(step),
                             dimensions=tuple(value['dimensions']), sequence=index,
                             next_building=70 + index, next_job=90 + index,
                             building_count=4 + index,
                             item=replace(original.plan.before.item, kind=index % 3 + 1,
                                          native_type=101 + index % 3))
            native = wire.Plan(batch.key(step), before)
            child = batch.effects.create(native, SOURCE, ADDRESS)
            if index < registered:
                batch.register(step)
            prepared = wire.Record(native, 'prepared', 'none')
            child.retain(placement_rpc.Reply(SOURCE, False, index + 1, record=prepared), prepared=True)
            child.append('dispatch', {'plan_digest': native.digest.hex()})
            insertion = replace(original.insertion, building=70 + index, job=90 + index,
                                item=step.item, kind=index % 3 + 1, pos=step.target)
            terminal = wire.Record(native, 'placed', 'none', before.expected_after(), insertion)
            child.retain(placement_rpc.Reply(SOURCE, False, index + 1, record=terminal))
        batch.audit()
        return selected, batch.id


def origin_from(path: Path, expected_id: str) -> completion.Origin:
    with batch_module.Batch(str(path), placement_rpc.Budget(60000)) as batch:
        return completion.Origin.from_batch(batch, expected_id)


def goal_from(origin: completion.Origin, **policy) -> completion.Goal:
    return completion.Goal(origin, plan.Goal(origin.receipts, policy.pop('deadline', TICK + 100), **policy))


def sample(goal: completion.Goal, tick: int = TICK + 1, **changes) -> plan.LinkedSample:
    return plan_sample(goal, tick, **changes)


def rewrite_child(child: completion.Child, transform) -> completion.Child:
    """Rehash each real placement transition so tests reach semantic validation."""
    raw, state = b'', None
    for line in child.raw.splitlines():
        saved = json.loads(line)
        kind, payload = transform(saved['kind'], saved['payload'])
        raw += placement_store.frame_bytes(state, kind, payload)
        state = placement_store.replay(raw)
    return replace(child, raw=raw)


def replace_record(child: completion.Child, record: wire.Record,
                   *, source: placement_rpc.Manifest = SOURCE, address=ADDRESS) -> completion.Child:
    raw = placement_store.frame_bytes(None, 'intent', placement_store.intent(record.plan, source, address))
    state = placement_store.replay(raw)
    raw += placement_store.frame_bytes(state, 'terminal',
                                      {'record_hex': record.raw.hex(), 'manifest': source.view()})
    return replace(child, raw=raw)


def reseal(origin: completion.Origin, *, manifest=None, children=None, entries=None) -> completion.Origin:
    """Preserve a valid index chain after an intentional semantic substitution."""
    manifest_raw = origin.manifest_raw if manifest is None else batch_module.seal(manifest)
    children = origin.children if children is None else children
    values = [batch_module.unseal(line) for line in origin.index_raw[len(batch_module.HEADER):].splitlines(keepends=True)]
    if entries is not None:
        values = entries
    raw = batch_module.HEADER
    for index, entry in enumerate(values):
        current = dict(entry, batch_id=batch_module.sha(manifest_raw),
                       previous=batch_module.sha(raw.splitlines(keepends=True)[-1]))
        if index < len(children):
            current['intent_sha256'] = batch_module.sha(children[index].raw.splitlines(keepends=True)[0])
        raw += batch_module.seal(current)
    return replace(origin, manifest_raw=manifest_raw, index_raw=raw, children=children)


class FurnitureCompletionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-furnishing-origin-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.batch_path = self.root / 'batch'
        self.plan, self.batch_id = create_batch(self.batch_path)
        self.origin = origin_from(self.batch_path, self.batch_id)
        self.goal = goal_from(self.origin)

    def reject_origin(self, **changes):
        with self.assertRaises((ValueError, TypeError)):
            replace(self.origin, **changes).encode()

    def test_full_32_step_origin_roundtrip_retains_original_plan_order_and_every_journal(self):
        path = self.root / 'maximum'
        selected, batch_id = create_batch(path, 32)
        original = origin_from(path, batch_id)
        raw = original.encode()
        restored = completion.Origin.decode(raw)
        self.assertEqual(restored, original)
        self.assertEqual(restored.encode(), raw)
        self.assertEqual(restored.batch_id, batch_id)
        self.assertEqual(restored.plan, selected)
        self.assertEqual(restored.source, SOURCE)
        self.assertEqual(restored.address, ADDRESS)
        self.assertEqual(len(restored.children), 32)
        self.assertEqual(tuple(wire.Record.decode(value).plan.key.rsplit('-', 1)[1]
                               for value in restored.receipts), tuple(s.name for s in selected.ordered))
        self.assertNotEqual(tuple(s.name for s in selected.ordered), tuple(s.name for s in selected.steps))
        for child, step in zip(restored.children, selected.ordered):
            stored = path / 'effects' / placement_store.filename('fb-' + 'ab' * 24 + '-' + step.name)
            self.assertEqual(child.raw, stored.read_bytes())
            self.assertEqual(child.file_identity, (stored.stat().st_dev, stored.stat().st_ino))
            self.assertEqual(placement_store.replay(child.raw).terminal.raw,
                             restored.receipts[selected.ordered.index(step)])
        self.assertEqual(completion.Goal.decode(goal_from(restored).encode()), goal_from(original))

    def test_complete_child_bijection_refuses_omission_duplication_reordering_and_foreign_names(self):
        children = self.origin.children
        for altered in ((), children[:-1], children + children[-1:], children[::-1],
                        (children[0], children[0], children[-1])):
            self.reject_origin(children=altered)
        for child in (replace(children[0], name='not-a-selected-child'),
                      replace(children[0], file_identity=(0, 1))):
            self.reject_origin(children=(child, *children[1:]))

    def test_unstarted_and_unregistered_placed_children_never_form_a_completion_origin(self):
        for name, placed, registered in (('incomplete', 2, 2), ('unregistered', 3, 2)):
            path = self.root / name
            _, batch_id = create_batch(path, placed=placed, registered=registered)
            with batch_module.Batch(str(path), placement_rpc.Budget(60000)) as batch:
                self.assertEqual(batch.audit()['placed'], placed)
                with self.assertRaises(ValueError):
                    completion.Origin.from_batch(batch, batch_id)
        with batch_module.Batch(str(self.batch_path), placement_rpc.Budget(60000)) as batch:
            with self.assertRaises(ValueError):
                completion.Origin.from_batch(batch, '0' * 64)

    def test_each_complete_child_is_independently_replayed_and_terminal_receipt_required(self):
        child = self.origin.children[-1]
        lines = child.raw.splitlines(keepends=True)
        for raw in (child.raw[:-1], child.raw + lines[-1], b''.join(lines[:-1]),
                    child.raw[:80] + bytes([child.raw[80] ^ 1]) + child.raw[81:]):
            self.reject_origin(children=(*self.origin.children[:-1], replace(child, raw=raw)))
        record = placement_store.replay(child.raw).terminal
        for phase, reason in (('cancelled', 'cancelled'), ('refused', 'stale'),
                              ('indeterminate', 'native_failure')):
            other = replace_record(child, wire.Record(record.plan, phase, reason))
            with self.assertRaises(ValueError):
                reseal(self.origin, children=(*self.origin.children[:-1], other)).encode()

    def test_rehashed_batch_source_and_terminal_manifest_substitution_are_refused(self):
        metadata = batch_module.unseal(self.origin.manifest_raw)
        for source in (dict(metadata['source'], generation=42),
                       dict(metadata['source'], df_version='different-df'),
                       dict(metadata['source'], dfhack_version='different-dfhack')):
            with self.assertRaises(ValueError):
                reseal(self.origin, manifest=dict(metadata, source=source)).encode()
        def new_terminal_generation(kind, payload):
            return kind, (dict(payload, manifest=dict(payload['manifest'], generation=42))
                          if kind == 'terminal' else payload)
        changed = rewrite_child(self.origin.children[-1], new_terminal_generation)
        self.assertEqual(placement_store.replay(changed.raw).terminal_manifest.generation, 42)
        self.reject_origin(children=(*self.origin.children[:-1], changed))
        path = self.batch_path / 'effects' / changed.name
        path.write_bytes(changed.raw)
        with batch_module.Batch(str(self.batch_path), placement_rpc.Budget(60000)) as batch:
            self.assertEqual(batch.audit()['status'], 'all_placed')
            with self.assertRaises(ValueError):
                completion.Origin.from_batch(batch, self.batch_id)

    def test_valid_replacement_receipt_cannot_change_exact_selection_key_or_endpoint(self):
        original = placement_store.replay(self.origin.children[-1].raw).terminal
        variants = []
        before = replace(original.plan.before, selection=replace(original.plan.before.selection, item=999))
        native = replace(original.plan, before=before)
        variants.append(replace(original, plan=native, after=before.expected_after(),
                                insertion=replace(original.insertion, item=999)))
        before = replace(original.plan.before, selection=replace(original.plan.before.selection, x=99))
        native = replace(original.plan, before=before)
        variants.append(replace(original, plan=native, after=before.expected_after(),
                                insertion=replace(original.insertion, pos=before.selection.target)))
        variants.append(replace(original, plan=replace(original.plan, key='another-original-key')))
        for record in variants:
            changed = replace_record(self.origin.children[-1], record)
            with self.assertRaises(ValueError):
                reseal(self.origin, children=(*self.origin.children[:-1], changed)).encode()
        changed = replace_record(self.origin.children[-1], original, address=('127.0.0.1', 5001))
        with self.assertRaises(ValueError):
            reseal(self.origin, children=(*self.origin.children[:-1], changed)).encode()

    def test_rehashed_index_must_register_every_original_step_exactly_once(self):
        entries = [batch_module.unseal(line) for line in self.origin.index_raw[len(batch_module.HEADER):].splitlines(keepends=True)]
        for altered in (entries[:-1], entries + entries[-1:], entries[::-1],
                        [dict(entries[0], file_identity=[0, 1]), *entries[1:]],
                        [dict(entries[0], step=entries[-1]['step']), *entries[1:]]):
            with self.assertRaises(ValueError):
                reseal(self.origin, entries=altered).encode()
        self.reject_origin(index_raw=self.origin.index_raw[:-1])

    def test_stopping_a_fully_placed_batch_does_not_change_its_completion_origin(self):
        with batch_module.Batch(str(self.batch_path), placement_rpc.Budget(60000), True) as batch:
            before = batch.audit()['inventory_digest']
            batch.stop()
            self.assertNotEqual(before, batch.audit()['inventory_digest'])
            self.origin.verify_batch(batch)
            self.assertEqual(completion.Origin.from_batch(batch, self.batch_id), self.origin)
        self.assertEqual(origin_from(self.batch_path, self.batch_id), self.origin)

    def test_byte_identical_metadata_inode_replacement_is_detected_after_reopen(self):
        for name in ('batch.json', 'steps.jsonl'):
            path = self.batch_path / name
            replacement = self.root / ('new-' + name)
            replacement.write_bytes(path.read_bytes())
            replacement.chmod(0o600)
            old = path.stat().st_ino
            os.replace(replacement, path)
            self.assertNotEqual(path.stat().st_ino, old)
            with batch_module.Batch(str(self.batch_path), placement_rpc.Budget(60000)) as batch:
                self.assertEqual(batch.audit()['status'], 'all_placed')
                with self.assertRaises(ValueError):
                    self.origin.verify_batch(batch)

    def test_live_custody_rechecks_detect_child_replacement_and_changed_manifest_bytes(self):
        for name in ('batch.json', 'child'):
            path = self.root / ('custody-' + name)
            _, batch_id = create_batch(path)
            original = origin_from(path, batch_id)
            with batch_module.Batch(str(path), placement_rpc.Budget(60000)) as batch:
                if name == 'child':
                    target = path / 'effects' / original.children[-1].name
                    replacement = self.root / 'new-child'
                    replacement.write_bytes(target.read_bytes())
                    replacement.chmod(0o600)
                    os.replace(replacement, target)
                else:
                    target = path / 'batch.json'
                    target.write_bytes(target.read_bytes().replace(b'"site":2', b'"site":3'))
                with self.assertRaises((ValueError, OSError)):
                    original.verify_batch(batch)

    def test_origin_encoding_is_bounded_canonical_and_commits_path_and_metadata_identity(self):
        raw = self.origin.encode()
        for data in (b'', raw[:7], raw[:len(raw) // 2], raw[:-1], raw + b'\0',
                     bytearray(raw), b'x' * (2 * 1024 * 1024 + 1)):
            with self.assertRaises((ValueError, TypeError)):
                completion.Origin.decode(data)
        for field, value in (('batch_path', 'relative/batch'), ('batch_path', str(self.batch_path) + '/..'),
                             ('manifest_identity', (True, 1)), ('index_identity', (-1, 1)),
                             ('children', list(self.origin.children))):
            self.reject_origin(**{field: value})
        for field, value in (('batch_path', str(self.root / 'other-batch')),
                             ('manifest_identity', (self.origin.manifest_identity[0], self.origin.manifest_identity[1] + 1)),
                             ('index_identity', (self.origin.index_identity[0], self.origin.index_identity[1] + 1))):
            self.assertNotEqual(replace(self.origin, **{field: value}).digest, self.origin.digest)

    def test_goal_requires_the_entire_original_receipt_selection_and_binds_policy(self):
        for receipts in (self.origin.receipts[:-1], self.origin.receipts[1:]):
            with self.assertRaises(ValueError):
                completion.Goal(self.origin, plan.Goal(receipts, TICK + 100)).encode()
        other = goal_from(self.origin, stable_samples=3)
        self.assertNotEqual(other.digest, self.goal.digest)
        self.assertNotEqual(self.goal.digest, self.goal.condition.digest)
        self.assertEqual(self.goal.receipts, self.goal.condition.receipts)
        self.assertEqual(self.goal.goals, self.goal.condition.goals)
        for attribute in ('deadline', 'interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'):
            self.assertEqual(getattr(self.goal, attribute), getattr(self.goal.condition, attribute))
        for data in (self.goal.encode()[:-1], self.goal.encode() + b'\0', self.goal.condition.encode()):
            with self.assertRaises(ValueError):
                completion.Goal.decode(data)

    def test_original_software_is_required_on_first_and_every_later_sample(self):
        good = sample(self.goal)
        wrong = receipt.Manifest(SOURCE.generation, 'different-df', SOURCE.dfhack_version)
        changed = replace(good, before=wrong, after=wrong,
                          operations=replace(wrong, generation=good.operations.generation))
        states = (plan.Progress(self.goal.digest),
                  completion.advance(completion.begin_read(plan.Progress(self.goal.digest)),
                                     self.goal, good, GUARD))
        for state in states:
            with self.assertRaises(ValueError):
                completion.advance(completion.begin_read(state), self.goal, changed, GUARD)
        with self.assertRaises(ValueError):
            completion.advance(plan.begin_read(plan.Progress(self.goal.condition.digest)), self.goal, good, GUARD)

    def test_all_original_targets_must_satisfy_one_global_stability_sequence(self):
        state = plan.Progress(self.goal.digest)
        for tick, missing_condition in ((TICK + 1, (72,)), (TICK + 2, (70,)), (TICK + 3, ())):
            state = completion.advance(completion.begin_read(state), self.goal,
                                       sample(self.goal, tick, uninstalled=missing_condition), GUARD)
            self.assertEqual(state.goal_digest, self.goal.digest)
            self.assertEqual(len(state.assessments), 3)
            self.assertFalse(state.terminal)
        self.assertEqual(state.streak, 1)
        state = completion.advance(completion.begin_read(state), self.goal, sample(self.goal, TICK + 4), GUARD)
        self.assertEqual((state.phase, state.streak), ('satisfied', 2))
        self.assertFalse(state.view()['placement_effect_discharged'])
        self.assertFalse(state.view()['current_usability_proven'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
