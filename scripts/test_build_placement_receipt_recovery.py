"""Exact positive-receipt recovery with real private files and injected RPC replies.

No placement method is called by recovery. These tests do not qualify native
DFHack execution or physical power loss. Existing strict codecs remain in use.
"""
from dataclasses import replace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import build_placement_client as client
import build_placement_rpc as rpc
import build_placement_store as store
import build_placement_wire as wire
from test_build_placement_store import ADDRESS, MANIFEST, plan, reply


class PositiveRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='dfmcp-positive-recovery-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.root.chmod(0o700)

    def owner(self, writable=True):
        return store.PlacementDirectory(str(self.root), rpc.Budget(60000), writable)

    def uncertain(self):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ADDRESS)
            journal.retain(reply(), prepared=True)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            journal.retain(reply('indeterminate'))
            self.assertEqual(journal.state.frames, 4)
            self.assertTrue(journal.state.pending)
            return journal.raw

    def test_reopen_accepts_one_exact_placed_receipt_and_preserves_uncertain_prefix(self):
        prefix = self.uncertain()
        with self.owner() as owner:
            journal = owner.get('golden')
            self.assertFalse(journal.fresh_intent)
            self.assertFalse(journal.prepared_this_owner)
            self.assertTrue(journal.retain_reconciliation(reply('placed')))
            self.assertEqual(journal.state.frames, 5)
            self.assertTrue(journal.raw.startswith(prefix))
            self.assertFalse(journal.state.pending)
            self.assertTrue(journal.state.dispatched)
            resolved = journal.raw
            self.assertFalse(journal.retain_reconciliation(reply('placed')))
            self.assertEqual(journal.raw, resolved)
            with self.assertRaises(wire.Rejected):
                journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            owner.ready('different-new-intent')
        with self.owner(False) as owner:
            self.assertEqual(owner.get('golden').raw, resolved)
            self.assertEqual(owner.inventory()['pending'], 0)
            self.assertFalse(owner.get('golden').state.view()['retry_permitted'])

    def test_nonapplication_or_wrong_original_key_never_replaces_unknown(self):
        prefix = self.uncertain()
        with self.owner() as owner:
            journal = owner.get('golden')
            candidates = [reply('prepared'), reply('expired'), reply('cancelled'),
                          reply('placed', key='other'),
                          replace(reply('placed'), manifest=replace(MANIFEST, df_version='different'))]
            for candidate in candidates:
                with self.subTest(candidate=candidate.record.phase), self.assertRaises(wire.Rejected):
                    journal.retain(candidate)
                self.assertEqual(journal.raw, prefix)
                self.assertTrue(journal.state.pending)
            self.assertFalse(journal.retain(reply('indeterminate')))
            self.assertEqual(journal.raw, prefix)
            with self.assertRaises(wire.Rejected):
                owner.ready('another')

    def test_failed_positive_receipt_sync_does_not_publish_or_acknowledge(self):
        prefix = self.uncertain()
        with self.owner() as owner:
            journal = owner.get('golden')
            old = journal.state
            with patch.object(store.os, 'fsync', side_effect=OSError('injected uncertain sync')):
                with self.assertRaises(OSError):
                    journal.retain_reconciliation(reply('placed'))
            self.assertTrue(owner.fenced)
            self.assertEqual(journal.state, old)
            self.assertEqual(journal.raw, prefix)
        # Complete bytes from an uncertain sync can survive. Their replay does
        # not assert that the failed call acknowledged durable storage.
        with self.owner(False) as owner:
            journal = owner.get('golden')
            self.assertFalse(journal.state.pending)
            self.assertEqual(journal.state.frames, 5)
            self.assertFalse(journal.state.view()['storage_acknowledged_this_call'])
            self.assertFalse(journal.prepared_this_owner)

    def test_partial_positive_receipt_never_replays_as_a_complete_outcome(self):
        self.uncertain()
        with self.owner() as owner:
            journal = owner.get('golden')
            original_write = store.os.write
            def partial(fd, data):
                original_write(fd, data[:3])
                raise OSError('injected torn positive receipt')
            with patch.object(store.os, 'write', side_effect=partial):
                with self.assertRaises(OSError):
                    journal.retain_reconciliation(reply('placed'))
            self.assertTrue(owner.fenced)
            self.assertTrue(journal.state.pending)
        with self.assertRaises((wire.Rejected, ValueError)):
            self.owner(False)

    def test_foreground_query_uses_original_selection_and_never_place_or_cancel(self):
        self.uncertain()
        authority = rpc.Authority(ADDRESS, b't' * 32)
        connection = MagicMock()
        connection.manifest = MANIFEST
        connection.query.return_value = reply('placed')
        with self.owner() as owner, patch.object(client, 'Client') as constructor, patch.object(rpc.Authority, 'guard'):
            constructor.return_value.__enter__.return_value = connection
            journal = owner.get('golden')
            result = client.recover(journal, authority)
            constructor.assert_called_once_with(authority, owner.budget, plan().before.selection)
            connection.query.assert_called_once_with(plan())
            connection.prepare.assert_not_called()
            connection.commit.assert_not_called()
            connection.cancel.assert_not_called()
            self.assertFalse(result['pending'])
            self.assertTrue(result['storage_acknowledged_this_call'])
            self.assertFalse(result['construction_completion_proven'])
            constructor.reset_mock()
            replayed = client.recover(journal, None)
            constructor.assert_not_called()
            self.assertFalse(replayed['native_contacted'])

    def test_cancel_absent_native_record_and_missing_authority_preserve_uncertainty(self):
        prefix = self.uncertain()
        with self.owner() as owner, patch.object(client, 'Client') as constructor, patch.object(rpc.Authority, 'guard'):
            journal = owner.get('golden')
            offline = client.recover(journal, None, True)
            with self.assertRaises(wire.Rejected):
                client.recover(journal, None)
            self.assertTrue(offline['pending'])
            self.assertFalse(offline['native_contacted'])
            constructor.assert_not_called()
            connection = MagicMock()
            connection.manifest = MANIFEST
            connection.query.return_value = rpc.Reply(MANIFEST, True, 1)
            constructor.return_value.__enter__.return_value = connection
            result = client.recover(journal, rpc.Authority(ADDRESS, b't' * 32))
            self.assertTrue(result['pending'])
            self.assertEqual(journal.raw, prefix)
            connection.commit.assert_not_called()
            connection.prepare.assert_not_called()
            connection.cancel.assert_not_called()

    def test_query_cli_requests_current_authority_for_an_uncertain_receipt(self):
        self.uncertain()
        authority = rpc.Authority(ADDRESS, b't' * 32)
        with patch.object(client.Authority, 'load', return_value=authority) as load, \
             patch.object(client, 'recover', side_effect=lambda journal, current, cancel: journal.state.view()) as recover, \
             patch.object(client.Authority, 'guard'), patch.object(client, 'write_output', return_value=0):
            self.assertEqual(client.main(['query', '--directory', str(self.root), '--key', 'golden']), 0)
            load.assert_called_once_with()
            self.assertIs(recover.call_args.args[1], authority)


    def test_maximal_text_and_five_frames_fit_reserved_capacity(self):
        value = wire.Plan('k' * 128, replace(plan().before, folder='\x01' * 512))
        manifest = rpc.Manifest(41, '\x01' * 128, '\x01' * 128)
        proof = reply('placed').record.insertion
        prepared = wire.Record(value, 'prepared', 'none')
        unknown = wire.Record(value, 'indeterminate', 'native_failure')
        placed = wire.Record(value, 'placed', 'none', value.before.expected_after(), proof)
        with self.owner() as owner:
            journal = owner.create(value, manifest, ADDRESS)
            journal.retain(rpc.Reply(manifest, False, 1, record=prepared), prepared=True)
            journal.append('dispatch', {'plan_digest': value.digest.hex()})
            journal.retain(rpc.Reply(manifest, True, 1, record=unknown))
            self.assertLessEqual(len(journal.raw) + store.MAX_FRAME, store.MAX_FILE)
        with self.owner() as owner:
            journal = owner.get(value.key)
            journal.retain_reconciliation(rpc.Reply(manifest, False, 1, record=placed))
            self.assertEqual(journal.state.frames, 5)
            self.assertLessEqual(len(journal.raw), store.MAX_FILE)
            self.assertFalse(journal.state.pending)
            self.assertLessEqual(len(client.bounded_output(client.packet('query', journal.state.view(), value))),
                                 client.MAX_OUTPUT)

    def test_every_partial_and_corrupt_positive_frame_is_rejected(self):
        prefix = self.uncertain()
        with self.owner() as owner:
            journal = owner.get('golden')
            journal.retain_reconciliation(reply('placed'))
            tail = journal.raw[len(prefix):]
            self.assertFalse(store.replay(journal.raw).pending)
        self.assertTrue(store.replay(prefix).pending)
        for n in range(len(tail)):
            if n:
                with self.subTest(prefix=n), self.assertRaises((ValueError, TypeError, KeyError)):
                    store.replay(prefix + tail[:n])
            bad = tail[:n] + bytes([tail[n] ^ 128]) + tail[n + 1:]
            with self.subTest(corruption=n), self.assertRaises((ValueError, TypeError, KeyError)):
                store.replay(prefix + bad)

    def test_recorded_source_floor_rejects_regressed_receipts_and_cross_incarnation_proof(self):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ADDRESS)
            journal.retain(reply('indeterminate', generation=43))
            prefix = journal.raw
            for name in ('placed', 'prepared', 'cancelled', 'expired', 'indeterminate'):
                candidate = reply(name, generation=42)
                with self.subTest(name=name), self.assertRaises(wire.Rejected):
                    journal.retain(candidate)
                self.assertEqual(journal.raw, prefix)
            for generation in (41, 42, 43):
                with self.subTest(generation=generation), self.assertRaises(wire.Rejected):
                    journal.retain_reconciliation(reply('placed', generation=generation))
            self.assertEqual(journal.raw, prefix)
            self.assertTrue(journal.state.pending)
            self.assertFalse(journal.state.dispatched)
            self.assertFalse(journal.prepared_this_owner)

    def test_positive_receipt_syncs_file_then_directory_before_acknowledgement(self):
        self.uncertain()
        with self.owner() as owner:
            journal = owner.get('golden')
            calls = []
            actual = store.os.fsync
            def sync(fd):
                calls.append(fd)
                self.assertTrue(journal.state.pending)
                actual(fd)
            with patch.object(store.os, 'fsync', side_effect=sync):
                self.assertTrue(journal.retain_reconciliation(reply('placed')))
            self.assertEqual(calls, [journal.fd, owner.fd])
            self.assertFalse(journal.state.pending)

    def test_uncertainty_reserves_next_receipt_before_append(self):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, ADDRESS)
            original = journal.raw
            candidate = reply('indeterminate')
            payload = {'record_hex': candidate.record.raw.hex(), 'manifest': candidate.manifest.view()}
            next_size = len(original) + len(store.frame_bytes(journal.state, 'terminal', payload))
            with patch.object(store, 'MAX_FILE', next_size + store.MAX_FRAME - 1), \
                 patch.object(store.os, 'write', side_effect=AssertionError('write before reservation')) as write:
                with self.assertRaises(wire.Rejected):
                    journal.retain(candidate)
                write.assert_not_called()
                self.assertEqual(journal.raw, original)
            self.assertTrue(journal.retain(candidate))


if __name__ == '__main__':
    unittest.main(verbosity=2)
