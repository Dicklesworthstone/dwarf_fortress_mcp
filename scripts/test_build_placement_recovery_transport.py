"""Actual TCP/CLI receipt recovery with joined native test peers and private files.

Beads df-dfhack-bridge-plane-c-pic.4/.5. The peer emits independent golden
receipt bytes. It is not a DFHack process or a generated-protobuf implementation.
"""
from contextlib import redirect_stdout
from dataclasses import replace
import io
import json
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading
import unittest
from unittest.mock import patch

import build_placement_client as cli
import build_placement_rpc as rpc
import build_placement_store as store
import build_placement_wire as wire
from test_build_placement_store import MANIFEST, plan, record, reply

SECRET = 's' * 32


class Peer:
    """Finite loopback peer; stop and join before releasing test-owned state."""
    def __init__(self, outcome='placed', generation=41, lose_once=False, hook=None):
        self.outcome, self.generation, self.lose_once, self.hook = outcome, generation, lose_once, hook
        self.retained = None
        self.calls = []
        self.effects = 0
        self.failure = None
        self.stop = threading.Event()
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(0.1)
        self.address = self.listener.getsockname()
        self.worker = threading.Thread(target=self.serve)

    def __enter__(self):
        self.worker.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.worker.join(6)
        self.listener.close()
        if self.worker.is_alive():
            raise AssertionError('native test peer did not quiesce')
        if self.failure is not None:
            raise AssertionError('native test peer failed') from self.failure

    def environment(self):
        return patch.dict(os.environ, {rpc.OPT_IN: '1', rpc.TOKEN: SECRET,
                          rpc.PLACE: '1', rpc.ENDPOINT: '%s:%s' % self.address}, clear=True)

    @staticmethod
    def receive(connection, count):
        out = bytearray()
        while len(out) < count:
            part = connection.recv(count - len(out))
            if not part:
                raise EOFError
            out.extend(part)
        return bytes(out)

    def serve(self):
        try:
            while not self.stop.is_set():
                try:
                    connection, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with connection:
                    connection.settimeout(5)
                    try:
                        self.exchange(connection)
                    except (EOFError, ConnectionResetError, BrokenPipeError):
                        pass
        except Exception as error:
            self.failure = error

    def exchange(self, connection):
        assert self.receive(connection, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        connection.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        methods = {}
        while not self.stop.is_set():
            method, width = struct.unpack('<h2xi', self.receive(connection, 8))
            assert 0 <= width <= 2048
            fields = rpc.decode(self.receive(connection, width))
            if method == 0:
                assert fields[2] == b'dfmcp.build.v1_19.Request'
                assert fields[3] == b'dfmcp.build.v1_19.Reply' and fields[4] == b'dfmcp_build_v1_19'
                name = fields[1].decode('ascii')
                assert name in rpc.METHODS and name not in methods.values()
                method = len(methods) + 2
                methods[method] = name
                response = {1: method}
            else:
                name = methods[method]
                self.calls.append(name)
                assert fields[1] == SECRET.encode() and fields[3] == 1 and fields[4] == 19
                response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 19, 6: self.generation,
                            7: MANIFEST.df_version.encode(), 8: MANIFEST.dfhack_version.encode(),
                            12: int(self.retained == 'indeterminate'), 13: int(self.retained is not None)}
                if name == 'ReadPlacement':
                    assert tuple(fields[n] for n in range(5, 10)) == plan().before.selection.values()
                    response[9] = plan().before.raw
                elif name != 'Handshake':
                    assert fields[10] == b'golden' and fields[12] == plan().digest
                    if name == 'PreparePlacement':
                        assert self.retained is None and fields[11] == plan().before.witness
                        self.retained = 'prepared'
                        response[11] = 0
                    elif name == 'CommitPlacement':
                        assert self.retained == 'prepared' and fields[13] == plan().token
                        self.effects += 1
                        self.retained = 'indeterminate'
                    elif name == 'QueryPlacement':
                        assert set(fields) == {1, 2, 3, 4, 10, 12}
                        if self.retained == 'indeterminate' and self.outcome == 'placed':
                            self.retained = 'placed'
                        if self.lose_once:
                            self.lose_once = False
                            return
                    else:
                        raise AssertionError('recovery must never cancel native work')
                    response[12] = int(self.retained == 'indeterminate')
                    response[13] = int(self.retained is not None)
                    if self.retained is not None:
                        response[10] = record(self.retained)
                    if name == 'QueryPlacement' and self.retained is not None:
                        if self.outcome == 'absent':
                            response.pop(10)
                        elif self.outcome in ('prepared', 'expired', 'cancelled'):
                            response[10] = record(self.outcome)
                        elif self.outcome == 'corrupt':
                            response[10] = record('placed')[:-1] + b'!'
                        elif self.outcome == 'wrong_key':
                            response[10] = record('placed', 'another')
                if self.hook is not None:
                    self.hook(name)
            payload = rpc.encode(response)
            connection.sendall(struct.pack('<h2xi', -1, len(payload)) + payload)


class RecoveryTransportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-recovery-tcp-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)

    def owner(self, writable=True):
        return store.PlacementDirectory(str(self.root), rpc.Budget(60000), writable)

    def seed(self, address, generation=41):
        with self.owner() as owner:
            journal = owner.create(plan(), MANIFEST, address)
            journal.retain(reply(), prepared=True)
            journal.append('dispatch', {'plan_digest': plan().digest.hex()})
            journal.retain(reply('indeterminate', generation=generation))
            return journal.raw

    def test_actual_start_reopen_revoked_placement_cli_query_and_offline_inspect(self):
        with Peer() as peer, peer.environment():
            with self.owner() as owner:
                result, _ = cli.start(owner, rpc.Authority.load(True), plan().before.selection,
                                      'golden', plan().digest.hex())
                self.assertTrue(result['pending'])
                prefix = owner.get('golden').raw
            os.environ[rpc.PLACE] = '0'
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                self.assertEqual(cli.main(['query', '--directory', str(self.root), '--key', 'golden']), 0)
            result = json.loads(stdout.getvalue())
            self.assertTrue(result['result']['storage_acknowledged_this_call'])
            self.assertFalse(result['result']['pending'])
            self.assertFalse(result['result']['construction_completion_proven'])
            with self.owner(False) as owner:
                journal = owner.get('golden')
                self.assertTrue(journal.raw.startswith(prefix))
                self.assertEqual(journal.state.frames, 5)
                self.assertFalse(journal.prepared_this_owner)
            with patch.dict(os.environ, {}, clear=True), redirect_stdout(io.StringIO()):
                self.assertEqual(cli.main(['inspect', '--directory', str(self.root), '--key', 'golden']), 0)
            self.assertEqual(peer.effects, 1)
            self.assertEqual(peer.calls.count('CommitPlacement'), 1)
            self.assertEqual(peer.calls.count('PreparePlacement'), 1)
            self.assertNotIn('CancelPlacement', peer.calls)

    def test_same_connection_positive_successor_never_restores_commit_permit(self):
        with Peer() as peer, peer.environment(), rpc.Client(rpc.Authority.load(), rpc.Budget(5000), plan().before.selection) as client:
            client.prepare(plan())
            self.assertEqual(client.commit(plan()).record.phase, 'indeterminate')
            self.assertEqual(client.query(plan()).record.phase, 'placed')
            self.assertIsNone(client._permit)
            with self.assertRaises(wire.Rejected):
                client.commit(plan())
            with self.assertRaises(wire.Rejected):
                client.prepare(plan())
            self.assertEqual(peer.effects, 1)

    def test_lost_positive_query_reply_reopens_and_never_repeats_placement(self):
        with Peer(lose_once=True) as peer, peer.environment():
            prefix = self.seed(peer.address)
            peer.retained = 'indeterminate'
            with self.owner() as owner:
                journal = owner.get('golden')
                with self.assertRaises(wire.Rejected):
                    cli.recover(journal, rpc.Authority.load())
                self.assertEqual(journal.raw, prefix)
            with self.owner() as owner:
                result = cli.recover(owner.get('golden'), rpc.Authority.load())
                self.assertFalse(result['pending'])
                self.assertEqual(owner.get('golden').state.frames, 5)
            self.assertEqual(peer.effects, 0)
            self.assertEqual(peer.calls, ['Handshake', 'QueryPlacement', 'Handshake', 'QueryPlacement'])

    def test_absent_and_repeated_uncertainty_keep_original_evidence_without_frames(self):
        for outcome in ('absent', 'indeterminate'):
            with self.subTest(outcome=outcome), Peer(outcome) as peer, peer.environment():
                directory = self.root / outcome
                directory.mkdir(mode=0o700)
                original_root, self.root = self.root, directory
                try:
                    prefix = self.seed(peer.address)
                    peer.retained = 'indeterminate'
                    for _ in range(3):
                        with self.owner() as owner:
                            journal = owner.get('golden')
                            result = cli.recover(journal, rpc.Authority.load())
                            self.assertTrue(result['pending'])
                            self.assertEqual(result['effect_status'], 'indeterminate')
                            self.assertEqual(result['effect']['receipt_digest'], reply('indeterminate').record.raw[-32:].hex())
                            self.assertEqual(result['native_record_present'], outcome != 'absent')
                            self.assertFalse(result['storage_acknowledged_this_call'])
                            self.assertEqual(journal.raw, prefix)
                finally:
                    self.root = original_root
                self.assertEqual(peer.effects, 0)

    def test_bad_positive_or_nonapplication_receipts_never_replace_unknown(self):
        for outcome in ('prepared', 'expired', 'cancelled', 'corrupt', 'wrong_key'):
            with self.subTest(outcome=outcome), Peer(outcome) as peer, peer.environment():
                directory = self.root / outcome
                directory.mkdir(mode=0o700)
                original_root, self.root = self.root, directory
                try:
                    prefix = self.seed(peer.address)
                    peer.retained = 'indeterminate'
                    with self.owner() as owner:
                        journal = owner.get('golden')
                        with self.assertRaises(wire.Rejected):
                            cli.recover(journal, rpc.Authority.load())
                        self.assertEqual(journal.raw, prefix)
                        self.assertTrue(journal.state.pending)
                    self.assertEqual(peer.effects, 0)
                finally:
                    self.root = original_root

    def test_recorded_source_floor_and_readonly_custody_reject_before_query(self):
        with Peer(generation=42) as peer, peer.environment():
            prefix = self.seed(peer.address, generation=43)
            peer.retained = 'indeterminate'
            with self.owner(False) as owner:
                with self.assertRaises(wire.Rejected):
                    cli.recover(owner.get('golden'), rpc.Authority.load())
            self.assertEqual(peer.calls, [])
            with self.owner() as owner:
                journal = owner.get('golden')
                with self.assertRaises(wire.Rejected):
                    cli.recover(journal, rpc.Authority.load())
                self.assertEqual(journal.raw, prefix)
                with self.assertRaises(wire.Rejected):
                    journal.retain(reply('placed', generation=42))
                self.assertEqual(journal.raw, prefix)
            self.assertEqual(peer.calls, ['Handshake'])

    def test_current_credential_revocation_before_publication_keeps_pending(self):
        def revoke(name):
            if name == 'QueryPlacement':
                os.environ[rpc.TOKEN] = 'x' * 32
        with Peer(hook=revoke) as peer, peer.environment():
            prefix = self.seed(peer.address)
            peer.retained = 'indeterminate'
            with self.owner() as owner:
                journal = owner.get('golden')
                with self.assertRaises(wire.Rejected):
                    cli.recover(journal, rpc.Authority.load())
                self.assertEqual(journal.raw, prefix)
                self.assertTrue(journal.state.pending)
            self.assertEqual(peer.effects, 0)


if __name__ == '__main__':
    unittest.main(verbosity=2)
