"""Execute the real receipt codec and private journal, never a game writer.

Bead df-dfhack-bridge-plane-c-pic.4: durable original-attempt reconciliation.
"""
from dataclasses import replace
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_placement_store as store
from build_placement_reconciliation import verify_reconciliation
from build_placement_rpc import Budget, Manifest, Reply
from build_placement_wire import (
    Capture, Insertion, Item, Plan, Record, Selection, Tile, canonical, digest,
)


def fixture(kind=1, wide=False):
    floor = Tile(2, 1, 3)
    before = Capture(7, 3, 100, 2, (32, 32, 8), 20, 30, 0,
        'r' * (512 if wide else 1), True, True, True,
        Selection(kind, 10, 8, 8, 5), (floor,) * 9,
        Item(presence=2, pos=(3, 3, 5), kind=kind, native_type=kind,
             subtype=-1, material=419, material_index=-1, on_ground=True, ground=floor))
    plan = Plan('k' * (128 if wide else 1), before)
    insertion = Insertion(20, 30, 10, kind, (8, 8, 5), 419, -1, 0, 1,
                          True, True, True, False)
    placed = Record(plan, 'placed', 'none', before.expected_after(), insertion)
    uncertain = Record(plan, 'indeterminate', 'native_failure')
    manifest = Manifest(7, 'd' * (128 if wide else 2), 'h' * (128 if wide else 2))
    return plan, uncertain, placed, manifest


def reply(record, manifest):
    return Reply(manifest, record.phase == 'indeterminate', 1, record=record)


def create_uncertain(owner, plan, uncertain, manifest):
    journal = owner.create(plan, manifest, ('127.0.0.1', 5000))
    journal.retain(reply(Record(plan, 'prepared', 'none'), manifest), prepared=True)
    journal.append('dispatch', {'plan_digest': plan.digest.hex()})
    journal.retain(reply(uncertain, manifest))
    return journal


def resolution(state, placed, manifest):
    return {'record_hex': placed.raw.hex(), 'manifest': manifest.view(),
            'previous_receipt': state.terminal.raw[-32:].hex()}


class ReceiptReconciliation(unittest.TestCase):
    def test_only_indeterminate_to_exact_placed_is_a_reconciliation(self):
        for kind in (1, 2, 3):
            plan, uncertain, placed, _ = fixture(kind)
            records = [Record(plan, 'prepared', 'none'), uncertain, placed,
                       Record(plan, 'refused', 'stale'), Record(plan, 'cancelled', 'cancelled')]
            for before in records:
                for after in records:
                    if before.phase == 'indeterminate' and after.phase == 'placed':
                        verify_reconciliation(before, after)
                    else:
                        with self.assertRaises(ValueError):
                            verify_reconciliation(before, after)

    def test_full_original_plan_and_capture_are_required_not_just_item_ids(self):
        plan, uncertain, _, _ = fixture()
        for changed in (replace(plan.before, tick=101), replace(plan.before, sequence=4),
                        replace(plan.before, generation=8), replace(plan.before, folder='other'),
                        replace(plan.before, item=replace(plan.before.item, quality=1))):
            p = Plan(plan.key, changed)
            insertion = Insertion(20, 30, 10, 1, (8, 8, 5), 419, -1, 0, 1, True, True, True, False)
            placed = Record(p, 'placed', 'none', changed.expected_after(), insertion)
            with self.assertRaises(ValueError):
                verify_reconciliation(uncertain, placed)

    def test_rechecksummed_wrong_after_or_insertion_still_refuses(self):
        _, uncertain, placed, _ = fixture()
        # Frozen records do not excuse skipping validation at a public boundary.
        for field, value in [('after', replace(placed.after, tick=101)),
                             ('insertion', replace(placed.insertion, exact_item_link=False)),
                             ('insertion', replace(placed.insertion, stage=1)),
                             ('insertion', replace(placed.insertion, building=21))]:
            forged = object.__new__(Record)
            for key, original in placed.__dict__.items():
                object.__setattr__(forged, key, value if key == field else original)
            with self.assertRaises(ValueError):
                verify_reconciliation(uncertain, forged)

    def test_append_retains_all_original_bytes_reopens_and_never_retries_key(self):
        for kind in (1, 2, 3):
            plan, uncertain, placed, manifest = fixture(kind)
            with tempfile.TemporaryDirectory() as path:
                with store.PlacementDirectory(path, Budget(60000), True) as owner:
                    journal = create_uncertain(owner, plan, uncertain, manifest)
                    original = journal.raw
                    with self.assertRaises(ValueError):
                        owner.ready('new')
                    # Ordinary terminal retention still cannot rewrite history.
                    with self.assertRaises(ValueError):
                        journal.retain(reply(placed, manifest))
                    self.assertTrue(journal.retain_reconciliation(reply(placed, manifest)))
                    self.assertTrue(journal.raw.startswith(original))
                    self.assertEqual(journal.state.reconciled_from.raw, uncertain.raw)
                    self.assertFalse(journal.state.pending)
                    self.assertEqual(journal.state.frames, 5)
                    self.assertEqual(owner.inventory()['pending'], 0)
                    owner.ready('new')
                    with self.assertRaises(ValueError):
                        owner.ready(plan.key)
                    self.assertFalse(journal.retain_reconciliation(reply(placed, manifest)))
                    final = journal.raw
                with store.PlacementDirectory(path, Budget(60000)) as owner:
                    journal = owner.get(plan.key)
                    self.assertEqual(journal.raw, final)
                    self.assertEqual(journal.state.terminal, placed)
                    self.assertEqual(journal.state.reconciled_from, uncertain)
                    self.assertFalse(journal.fresh_intent or journal.prepared_this_owner)
                    disclosure = journal.state.view()['reconciliation']
                    self.assertEqual(disclosure['original_indeterminate_receipt'], uncertain.raw[-32:].hex())
                    self.assertFalse(disclosure['new_placement_dispatched'])

    def test_all_new_frame_truncations_remain_uncertain_or_refuse(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                old = journal.raw
                extra = store.frame_bytes(journal.state, 'reconciled', resolution(journal.state, placed, manifest))
                self.assertTrue(store.replay(old).pending)
                for size in range(1, len(extra)):
                    with self.assertRaises(ValueError, msg=f'truncation {size}'):
                        store.replay(old + extra[:size])
                self.assertFalse(store.replay(old + extra).pending)
                self.assertEqual(Path(path, store.filename(plan.key)).read_bytes(), old)

    def test_every_recovery_frame_byte_mutation_and_duplicate_frame_refuse(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                extra = store.frame_bytes(journal.state, 'reconciled', resolution(journal.state, placed, manifest))
                for index in range(len(extra)):
                    bad = bytearray(extra)
                    bad[index] ^= 1
                    with self.assertRaises((ValueError, UnicodeError), msg=f'byte {index}'):
                        store.replay(journal.raw + bytes(bad))
                with self.assertRaises(ValueError):
                    store.replay(journal.raw + extra + extra)

    def test_wrong_receipt_source_and_transition_refuse_before_any_write(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                old = journal.raw
                for field, value in [('previous_receipt', '0' * 64),
                                     ('manifest', replace(manifest, generation=8).view()),
                                     ('manifest', replace(manifest, df_version='changed').view()),
                                     ('record_hex', Record(plan, 'cancelled', 'cancelled').raw.hex()),
                                     ('record_hex', Record(plan, 'refused', 'stale').raw.hex())]:
                    payload = resolution(journal.state, placed, manifest)
                    payload[field] = value
                    with self.assertRaises(ValueError):
                        journal.append('reconciled', payload)
                    self.assertEqual(Path(path, store.filename(plan.key)).read_bytes(), old)
                self.assertTrue(journal.state.pending)

    def test_uncertain_repeat_is_noop_and_resolved_outcomes_cannot_change(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                raw = journal.raw
                self.assertFalse(journal.retain_reconciliation(reply(uncertain, manifest)))
                self.assertEqual(journal.raw, raw)
                journal.retain_reconciliation(reply(placed, manifest))
                for value in [uncertain, Record(plan, 'cancelled', 'cancelled'),
                              Record(plan, 'refused', 'stale')]:
                    with self.assertRaises(ValueError):
                        journal.retain_reconciliation(reply(value, manifest))

    def test_reconciliation_has_no_prepared_or_dispatch_permission(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                create_uncertain(owner, plan, uncertain, manifest)
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = owner.get(plan.key)
                journal.retain_reconciliation(reply(placed, manifest))
                self.assertFalse(journal.fresh_intent or journal.prepared_this_owner)
                for kind, payload in [('dispatch', {'plan_digest': plan.digest.hex()}),
                                      ('prepared', {'record_hex': Record(plan, 'prepared', 'none').raw.hex(),
                                                    'manifest': manifest.view()})]:
                    with self.assertRaises(ValueError):
                        journal.append(kind, payload)

    def test_read_only_recovery_cannot_append_and_expiry_prevents_persistence(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                old = create_uncertain(owner, plan, uncertain, manifest).raw
            with store.PlacementDirectory(path, Budget(60000)) as owner:
                with self.assertRaises(ValueError):
                    owner.get(plan.key).retain_reconciliation(reply(placed, manifest))
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = owner.get(plan.key)
                owner.budget.deadline = 0
                with self.assertRaises(ValueError):
                    journal.retain_reconciliation(reply(placed, manifest))
            self.assertEqual(Path(path, store.filename(plan.key)).read_bytes(), old)

    def test_partial_append_fences_and_reopen_never_repairs(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                old = journal.raw
                def torn(fd, raw):
                    os.write(fd, raw[:17])
                    raise OSError('injected short write')
                with patch.object(store, 'write_all', torn):
                    with self.assertRaises(OSError):
                        journal.retain_reconciliation(reply(placed, manifest))
                self.assertTrue(owner.fenced)
                self.assertTrue(journal.state.pending)
            file = Path(path, store.filename(plan.key))
            torn_bytes = file.read_bytes()
            self.assertTrue(torn_bytes.startswith(old))
            with self.assertRaises(ValueError):
                store.PlacementDirectory(path, Budget(60000), True)
            self.assertEqual(file.read_bytes(), torn_bytes)

    def test_lost_sync_ack_preserves_history_and_complete_surviving_frame_replays(self):
        plan, uncertain, placed, manifest = fixture()
        for failed_sync in (1, 2):
            with tempfile.TemporaryDirectory() as path:
                with store.PlacementDirectory(path, Budget(60000), True) as owner:
                    journal = create_uncertain(owner, plan, uncertain, manifest)
                    old = journal.raw
                    count = 0
                    def sync(_fd):
                        nonlocal count
                        count += 1
                        if count == failed_sync:
                            raise OSError('lost sync acknowledgement')
                    with patch.object(store.os, 'fsync', sync):
                        with self.assertRaises(OSError):
                            journal.retain_reconciliation(reply(placed, manifest))
                    self.assertTrue(owner.fenced)
                    self.assertTrue(journal.state.pending)
                with store.PlacementDirectory(path, Budget(60000)) as owner:
                    journal = owner.get(plan.key)
                    self.assertTrue(journal.raw.startswith(old))
                    self.assertEqual(journal.state.reconciled_from, uncertain)
                    self.assertEqual(journal.state.terminal, placed)
                    self.assertFalse(journal.state.view()['storage_acknowledged_this_call'])

    def test_changed_backing_and_final_readback_fail_without_publishing_success(self):
        plan, uncertain, placed, manifest = fixture()
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                original_read = journal._read
                before = journal.raw
                def read():
                    raw = original_read()
                    return raw if len(raw) == len(before) else b'not the acknowledged bytes'
                with patch.object(journal, '_read', read):
                    with self.assertRaises(ValueError):
                        journal.retain_reconciliation(reply(placed, manifest))
                self.assertTrue(owner.fenced)
                self.assertTrue(journal.state.pending)

    def test_maximum_width_reconciliation_fits_existing_storage_bounds(self):
        plan, uncertain, placed, manifest = fixture(wide=True)
        with tempfile.TemporaryDirectory() as path:
            with store.PlacementDirectory(path, Budget(60000), True) as owner:
                journal = create_uncertain(owner, plan, uncertain, manifest)
                journal.retain_reconciliation(reply(placed, manifest))
                self.assertLessEqual(len(journal.raw), store.MAX_FILE)
                self.assertTrue(all(len(line) <= store.MAX_FRAME for line in journal.raw.splitlines(keepends=True)))
                self.assertEqual(store.replay(journal.raw).terminal.raw, placed.raw)


if __name__ == '__main__':
    unittest.main()
