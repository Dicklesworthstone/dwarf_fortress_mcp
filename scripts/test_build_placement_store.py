"""Real POSIX furniture journal custody and crash-prefix checks; no live game.

Native receipt fixtures come from the independently constructed engine corpus.
These tests make no DFHack SDK, power-loss, runtime or compatibility claim.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import build_placement_rpc as rpc
import build_placement_store as s
import build_placement_wire as w

ROOT = Path(__file__).resolve().parents[1]
VECTORS = json.loads((ROOT / 'bridge/common/tests/fixtures/build_placement_v1_19.json').read_text())
MANIFEST = rpc.Manifest(41, 'fake-df', 'fake-dfhack')
ADDRESS = ('127.0.0.1', 5000)


def plan(key='golden'):
    return w.Plan(key, w.Capture.decode(bytes.fromhex(VECTORS['capture'])))


def record(name='prepared', key='golden'):
    """Rebind only the explicitly keyed fields of independently checked bytes."""
    value = bytes.fromhex(VECTORS[name])
    before = bytes.fromhex(VECTORS['capture'])
    prefix = 8 + 2 + len('golden') + 2 + len(before) + 32 + 16
    selected = plan(key)
    framed_key = struct.pack('>H', len(key)) + key.encode('ascii')
    framed_capture = struct.pack('>H', len(before)) + before
    raw = b'DFMBR019' + framed_key + framed_capture + selected.digest + selected.token + value[prefix:-32]
    return raw + hashlib.sha256(b'dfmcp-build-receipt/1\0' + raw).digest()


def reply(name='prepared', key='golden', generation=41):
    value = w.Record.decode(record(name, key))
    return rpc.Reply(replace(MANIFEST, generation=generation), name == 'indeterminate', 1, record=value)


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-build-placement-store-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)

    def owner(self, writable=True):
        return s.PlacementDirectory(str(self.root), rpc.Budget(10000), writable)

    def create(self, outcome=None, key='golden'):
        with self.owner() as owner:
            journal = owner.create(plan(key), MANIFEST, ADDRESS)
            if outcome is not None:
                journal.retain(reply(outcome, key))
        return self.root / s.filename(key)

    def prepared(self, owner):
        journal = owner.create(plan(), MANIFEST, ADDRESS)
        journal.retain(reply(), prepared=True)
        return journal

    def test_complete_journal_replay_and_terminal_idempotence(self):
        with self.owner() as owner:
            journal = self.prepared(owner)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertTrue(journal.retain(reply('placed')))
            original = journal.raw
            self.assertFalse(journal.retain(reply('placed')))
            self.assertEqual(journal.raw, original)
            self.assertEqual(journal.state.frames, 4)
        with self.owner(False) as owner:
            state = owner.get('golden').state
            self.assertTrue(state.dispatched)
            self.assertFalse(state.pending)
            self.assertEqual(state.terminal.phase, 'placed')
            self.assertEqual(state.terminal.raw, bytes.fromhex(VECTORS['placed']))
            self.assertFalse(state.view()['storage_acknowledged_this_call'])
            self.assertFalse(state.view()['construction_completion_proved'])

    def test_every_incomplete_prefix_and_one_byte_corruption(self):
        raw = self.create('placed').read_bytes()
        boundaries = {i + 1 for i, byte in enumerate(raw) if byte == 10}
        for offset in range(len(raw)):
            if offset in boundaries:
                self.assertTrue(s.replay(raw[:offset]).pending)
            else:
                with self.subTest(prefix=offset), self.assertRaises((ValueError, TypeError, KeyError)):
                    s.replay(raw[:offset])
            corrupted = raw[:offset] + bytes([raw[offset] ^ 128]) + raw[offset + 1:]
            with self.subTest(byte=offset), self.assertRaises((ValueError, TypeError, KeyError)):
                s.replay(corrupted)

    def test_rehashed_invalid_transitions_and_receipts_fail_before_write(self):
        state = s.replay(s.frame_bytes(None, 'intent', s.intent(plan(), MANIFEST, ADDRESS)))
        for kind, payload in (
            ('dispatch', {'plan_digest': plan().digest.hex()}),
            ('prepared', {'record_hex': record('placed').hex(), 'manifest': MANIFEST.view()}),
            ('terminal', {'record_hex': record().hex(), 'manifest': MANIFEST.view()}),
            ('terminal', {'record_hex': record('placed', 'wrong').hex(), 'manifest': MANIFEST.view()}),
            ('terminal', {'record_hex': record('placed').hex(), 'manifest': replace(MANIFEST, generation=40).view()}),
        ):
            with self.subTest(kind=kind), self.assertRaises(w.Rejected):
                s.frame_bytes(state, kind, payload)
        with self.owner() as owner:
            journal = self.prepared(owner)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            original = journal.raw
            with self.assertRaises(w.Rejected):
                journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertEqual(journal.raw, original)

    def test_indeterminate_is_immutable_and_remains_pending(self):
        with self.owner() as owner:
            journal = self.prepared(owner)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertTrue(journal.retain(reply('indeterminate')))
            self.assertTrue(journal.state.pending)
            original = journal.raw
            self.assertFalse(journal.retain(reply('indeterminate')))
            for changed in ('prepared', 'placed', 'expired', 'cancelled'):
                with self.subTest(changed=changed), self.assertRaises(w.Rejected):
                    journal.retain(reply(changed))
            self.assertEqual(journal.raw, original)
            with self.assertRaises(w.Rejected):
                owner.ready('another')
            inventory = owner.inventory()
            self.assertEqual((inventory['pending'], inventory['operator_attention']), (1, 1))
        with self.owner() as owner:
            self.assertEqual(owner.get('golden').state.terminal.raw, record('indeterminate'))
            with self.assertRaises(w.Rejected):
                owner.ready('another')

    def test_recovery_query_cannot_restore_dispatch_authority(self):
        with self.owner() as owner:
            self.prepared(owner)
        with self.owner() as owner:
            journal = owner.get('golden')
            raw = journal.raw
            self.assertFalse(journal.retain(reply()))
            with self.assertRaises(w.Rejected):
                journal.retain(reply(), prepared=True)
            with self.assertRaises(w.Rejected):
                journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertEqual(journal.raw, raw)
            # A cancellation receipt may retire an effect-free preparation.
            self.assertTrue(journal.retain(reply('cancelled')))
            owner.ready('new')

    def test_public_record_substitution_and_software_change_are_rejected(self):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ADDRESS)
            forged = reply()
            # Even deliberate mutation of frozen Python evidence is revalidated
            # at the storage boundary, before a frame is written.
            object.__setattr__(forged.record, 'phase', 'placed')
            with self.assertRaises(w.Rejected):
                journal.retain(forged, prepared=True)
            changed = replace(reply('placed'), manifest=replace(MANIFEST, df_version='changed'))
            with self.assertRaises(w.Rejected):
                journal.retain(changed)
            self.assertEqual(journal.state.frames, 1)

    def test_complete_directory_pending_fence_and_permanent_key_retention(self):
        self.create('cancelled')
        with self.owner() as owner:
            owner.ready('new')
            with self.assertRaises(w.Rejected):
                owner.ready('golden')
            owner.create(plan('new'), MANIFEST, ADDRESS)
            with self.assertRaises(w.Rejected):
                owner.ready('another')
            self.assertEqual(owner.inventory()['total'], 2)

    def test_pending_first_inventory_has_bound_stale_continuations(self):
        with self.owner() as owner:
            for number in range(4):
                key = f'a{number}'
                journal = owner.create(plan(key), MANIFEST, ADDRESS)
                journal.retain(reply('cancelled', key))
            journal = owner.create(plan('z-pending'), MANIFEST, ADDRESS)
            first = owner.inventory(2)
            self.assertEqual((first['total'], first['pending']), (5, 1))
            self.assertEqual(first['rows'][0]['key'], 'z-pending')
            second = owner.inventory(2, first['continuation'])
            self.assertEqual((second['total'], second['pending']), (5, 1))
            with self.assertRaises(w.Rejected):
                owner.inventory(3, first['continuation'])
            journal.retain(reply('cancelled', 'z-pending'))
            with self.assertRaises(w.Rejected):
                owner.inventory(2, first['continuation'])
            for limit in (0, 65, True):
                with self.assertRaises(w.Rejected):
                    owner.inventory(limit)

    def test_modes_links_special_entries_and_parent_replacement(self):
        path = self.create()
        path.chmod(0o400)
        with self.assertRaises((OSError, w.Rejected)):
            self.owner()
        path.chmod(0o600)
        os.link(path, self.root / s.filename('other'))
        with self.assertRaises(w.Rejected):
            self.owner()
        for kind in ('symlink', 'fifo'):
            root = self.root / kind
            root.mkdir(mode=0o700)
            target = root / s.filename('golden')
            if kind == 'symlink':
                target.symlink_to(path)
            else:
                os.mkfifo(target, 0o600)
            with self.assertRaises((OSError, w.Rejected)):
                s.PlacementDirectory(str(root), rpc.Budget(1000), True)
        root = self.root / 'replace'
        root.mkdir(mode=0o700)
        with s.PlacementDirectory(str(root), rpc.Budget(1000), True) as owner:
            root.rename(self.root / 'moved')
            root.mkdir(mode=0o700)
            with self.assertRaises(w.Rejected):
                owner.check()

    def test_directory_modes_and_symlinked_ancestor_refused(self):
        self.root.chmod(0o500)
        with self.assertRaises(w.Rejected):
            self.owner()
        self.root.chmod(0o700)
        child = self.root / 'actual'
        child.mkdir(mode=0o700)
        link = self.root / 'alias'
        link.symlink_to(child, target_is_directory=True)
        with self.assertRaises((OSError, w.Rejected)):
            s.PlacementDirectory(str(link), rpc.Budget(1000), True)

    def test_same_size_tampering_and_new_membership_block_an_owner(self):
        path = self.create()
        with self.owner() as owner:
            raw = path.read_bytes()
            path.write_bytes(raw.replace(b'fake-df', b'fuke-df'))
            with self.assertRaises(w.Rejected):
                owner.check()
        other = self.root / 'other'
        other.mkdir(mode=0o700)
        with s.PlacementDirectory(str(other), rpc.Budget(1000), True) as owner:
            (other / 'extra.placement').write_bytes(b'bad')
            with self.assertRaises(w.Rejected):
                owner.ready('golden')

    def test_real_subprocess_directory_lock_covers_offline_inspection(self):
        self.create('placed')
        program = ('import build_placement_store as s; import build_placement_rpc as r; '
                   'import sys; owner=s.PlacementDirectory(sys.argv[1],r.Budget(1000)); '
                   'print(owner.get("golden").state.terminal.phase); owner.close()')
        command = [sys.executable, '-c', program, str(self.root)]
        environment = {'PYTHONPATH': str(ROOT / 'scripts')}
        with self.owner() as owner:
            blocked = subprocess.run(command, capture_output=True, text=True, timeout=5, env=environment)
            self.assertNotEqual(blocked.returncode, 0)
            owner.check()
        completed = subprocess.run(command, capture_output=True, text=True, timeout=5, env=environment)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout.strip(), 'placed')

    def test_short_writes_finish_and_partial_creation_remains_fenced(self):
        actual = os.write
        with self.owner() as owner, patch.object(s.os, 'write', side_effect=lambda fd, raw: actual(fd, raw[:7])):
            journal = owner.create(plan(), MANIFEST, ADDRESS)
            self.assertEqual(journal.state.plan, plan())
        root = self.root / 'partial'
        root.mkdir(mode=0o700)
        calls = 0
        def fail(fd, raw):
            nonlocal calls
            calls += 1
            if calls == 1:
                return actual(fd, raw[:20])
            raise OSError('injected partial write')
        with s.PlacementDirectory(str(root), rpc.Budget(1000), True) as owner:
            with patch.object(s.os, 'write', side_effect=fail), self.assertRaises(OSError):
                owner.create(plan(), MANIFEST, ADDRESS)
            self.assertTrue(owner.fenced)
        self.assertEqual((root / s.filename('golden')).stat().st_size, 20)
        with self.assertRaises(w.Rejected):
            s.PlacementDirectory(str(root), rpc.Budget(1000), True)

    def test_dispatch_and_receipt_sync_failures_never_acknowledge(self):
        with self.owner() as owner:
            journal = self.prepared(owner)
            with patch.object(s.os, 'fsync', side_effect=OSError('injected dispatch sync failure')):
                with self.assertRaises(OSError):
                    journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            self.assertTrue(owner.fenced)
        with self.owner() as owner:
            journal = owner.get('golden')
            self.assertTrue(journal.state.dispatched)
            with patch.object(s.os, 'fsync', side_effect=OSError('injected receipt sync failure')):
                with self.assertRaises(OSError):
                    journal.retain(reply('placed'))
            self.assertTrue(owner.fenced)
        with self.owner(False) as owner:
            state = owner.get('golden').state
            self.assertFalse(state.pending)
            self.assertFalse(state.view()['storage_acknowledged_this_call'])

    def test_readonly_expired_and_bad_manifest_owners_never_create(self):
        with self.owner(False) as owner:
            with self.assertRaises(w.Rejected):
                owner.create(plan(), MANIFEST, ADDRESS)
        with self.owner() as owner:
            with self.assertRaises(w.Rejected):
                owner.create(plan(), replace(MANIFEST, generation=42), ADDRESS)
            owner.budget.deadline = 0
            with self.assertRaises(w.Rejected):
                owner.create(plan(), MANIFEST, ADDRESS)
        self.assertEqual(list(self.root.iterdir()), [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
