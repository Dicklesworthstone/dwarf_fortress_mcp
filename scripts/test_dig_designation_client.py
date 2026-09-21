#!/usr/bin/env python3
"""Execute the dig developer client against joined loopback protocol doubles.

These are real Python/filesystem/socket tests, not a real DFHack SDK or game.
"""
from __future__ import annotations
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import dig_designation_client as d
from test_dig_designation_native import reference_vectors

ROOT = Path(__file__).resolve().parents[1]
REGION = dict(zip(d.REGION_KEYS, (15, 15, 2, 2, 2)))
MANIFEST = {'generation': 7, 'df_version': 'test-df', 'dfhack_version': 'test-dfhack'}
TOKEN = b's' * 32
VECTORS = reference_vectors()


def intent(address: str = '127.0.0.1:5000') -> dict:
    return d.build_intent(address, 'dig-001', REGION, False, VECTORS['observation'], MANIFEST)


def rehash(raw: bytes) -> bytes:
    out = bytearray(raw)
    out[172:204] = d.digest(b'dfmcp-dig-designation-receipt/1', raw[8:16] + raw[204:]
                            + raw[85:133] + raw[133:172])
    return bytes(out)


def outcome(state: int, reason: int = 0, known: int | None = None) -> bytes:
    raw = bytearray(VECTORS['designated'] if state == 2 else VECTORS['prepared'])
    raw[133] = state; raw[134] = reason
    raw[135] = int(state == 2) if known is None else known
    if state in (2, 4):
        return rehash(bytes(raw))
    return bytes(raw)


class FakeGame:
    def __init__(self):
        self.calls = []
        self.observation = VECTORS['observation']
        self.record = None
        self.replayed = False
        self.commit_mode = 'normal'
        self.fail = None
        self.source_generation = 7
        self.notifications = []
        self.binding_alias = False
        self.after_prepare = None
        self.after_commit = None


class Peer:
    """Owned/joined single-connection server. Exact method/request checks retained."""
    def __init__(self, game: FakeGame, connections: int = 1):
        self.game = game
        self.connections = connections
        self.errors = []
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0)); self.listener.listen(1); self.listener.settimeout(3)
        self.address = f'127.0.0.1:{self.listener.getsockname()[1]}'
        self.connection = None
        self.thread = threading.Thread(target=self.serve)

    @staticmethod
    def exact(sock: socket.socket, size: int) -> bytes:
        result = bytearray()
        while len(result) < size:
            part = sock.recv(size - len(result))
            if not part:
                raise EOFError
            result += part
        return bytes(result)

    def send(self, raw: bytes) -> None:
        # Fragment real TCP writes, without an unbounded loop or sleeps.
        for start in range(0, len(raw), 7):
            self.connection.sendall(raw[start:start + 7])

    def response(self, fields: dict) -> None:
        raw = d.encode(fields)
        self.send(struct.pack('<h2xi', -1, len(raw)) + raw)

    def serve(self) -> None:
        try:
            for _ in range(self.connections):
                self.serve_one()
        finally:
            self.listener.close()

    def serve_one(self) -> None:
        try:
            sock, _ = self.listener.accept(); self.connection = sock; sock.settimeout(3)
            with sock:
                assert self.exact(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
                self.send(b'DFHack!\n' + struct.pack('<i', 1))
                index = 0
                while True:
                    kind, size = struct.unpack('<h2xi', self.exact(sock, 8))
                    assert 0 <= size <= 2048
                    fields = d.decode(self.exact(sock, size), 14)
                    if kind == 0:
                        assert index < 6
                        assert fields == {1: d.METHODS[index].encode(), 2: b'dfmcp.dig.v1_16.Request',
                                          3: b'dfmcp.dig.v1_16.Reply', 4: b'dfmcp_dig_v1_16'}
                        self.response({1: 2 if self.game.binding_alias else 2 + index})
                        index += 1; continue
                    assert index == 6 and 2 <= kind <= 7
                    name = d.METHODS[kind - 2]
                    self.game.calls.append(name)
                    assert fields[1] == TOKEN and 16 <= len(fields[2]) <= 64 and fields[3] == 1 and fields[4] == 16
                    extras = set(fields) - {1, 2, 3, 4}
                    assert extras == {'Handshake': set(), 'ReadDesignation': set(range(5, 10)),
                        'PrepareDesignation': set(range(5, 14)), 'CommitDesignation': {11, 13, 14},
                        'CancelDesignation': {11, 13, 14}, 'QueryDesignation': {11, 13}}[name]
                    if name in ('ReadDesignation', 'PrepareDesignation'):
                        assert tuple(fields[i] for i in range(5, 10)) == tuple(REGION[k] for k in d.REGION_KEYS)
                    if name in ('PrepareDesignation', 'CommitDesignation', 'CancelDesignation', 'QueryDesignation'):
                        expected = intent(self.address)
                        assert fields[11] == b'dig-001' and fields[13] == bytes.fromhex(expected['plan_digest'])
                        if 12 in fields:
                            assert fields[12] == bytes.fromhex(expected['witness']) and fields[10] == 0
                        if 14 in fields:
                            assert fields[14] == bytes.fromhex(expected['prepare_token'])
                    response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 16, 6: self.game.source_generation,
                                7: MANIFEST['df_version'].encode(), 8: MANIFEST['dfhack_version'].encode()}
                    if name == 'ReadDesignation':
                        response[9] = self.game.observation
                    if name == 'PrepareDesignation':
                        if self.game.record is None:
                            self.game.record = VECTORS['prepared']
                        response[10] = self.game.record; response[11] = int(self.game.replayed)
                        if self.game.after_prepare:
                            self.game.after_prepare()
                    if name == 'CommitDesignation':
                        self.game.record = outcome(1) if self.game.commit_mode == 'unknown' else VECTORS['designated']
                        if self.game.after_commit:
                            self.game.after_commit()
                        if self.game.commit_mode == 'lost':
                            return
                        response[10] = self.game.record
                        if self.game.commit_mode == 'forged':
                            bad = bytearray(self.game.record); bad[139] ^= 1
                            response[10] = rehash(bytes(bad))
                        if self.game.commit_mode == 'prepared':
                            response[10] = VECTORS['prepared']
                    if name == 'QueryDesignation' and self.game.record is not None:
                        response[10] = self.game.record
                    if name == 'CancelDesignation':
                        if self.game.record is None or self.game.record == VECTORS['prepared']:
                            self.game.record = VECTORS['cancelled']
                        response[10] = self.game.record
                    if self.game.fail and self.game.fail[0] == name:
                        field, value = self.game.fail[1:]; response[field] = value
                    for size in self.game.notifications:
                        self.send(struct.pack('<h2xi', -3, size) + bytes(size))
                    self.response(response)
        except (EOFError, ConnectionResetError, BrokenPipeError):
            pass
        except BaseException as cause:
            self.errors.append(cause)

    def __enter__(self) -> Peer:
        self.thread.start(); return self

    def __exit__(self, *_args) -> None:
        if self.connection is not None:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.thread.join(5)
        if self.thread.is_alive():
            self.listener.close(); self.thread.join(5)
        if self.thread.is_alive():
            raise AssertionError('loopback peer failed to quiesce')
        if self.errors:
            raise AssertionError(f'loopback peer assertion: {self.errors[0]!r}')


class DigClientTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='dfmcp-dig-client-')
        self.root = Path(self.temp.name).resolve(); os.chmod(self.root, 0o700)
        self.path = self.root / 'intent.json'

    def tearDown(self):
        self.temp.cleanup()

    def write(self, value: dict | None = None) -> None:
        with d.capsule(self.path, intent() if value is None else value):
            pass

    def case_path(self, name: str) -> Path:
        # Independent protocol faults must not bypass the new store-wide fence.
        directory = self.root / name
        directory.mkdir(mode=0o700)
        return directory / 'intent.json'

    def start(self, client: d.Client) -> dict:
        value = intent(client.address)
        return d.start(client, self.path, 'dig-001', REGION, False, value['witness'], value['plan_digest'])

    def test_actual_cpp_vectors_and_precise_expected_readback(self):
        for name, raw in VECTORS.items():
            self.assertEqual(bytes.fromhex((ROOT / f'tests/native/dig_designation/vectors/{name}.hex').read_text()), raw)
        value = intent(); o = d.observation(VECTORS['observation'], REGION)
        self.assertEqual(len(o['cells']), 48); self.assertEqual(d.blockers(o, False), [])
        for name, state in [('prepared', 'prepared'), ('designated', 'designated'), ('cancelled', 'refused')]:
            self.assertEqual(d.effect(VECTORS[name], value)['state'], state)
        after = d.observation(d.expected_after(VECTORS['observation'], REGION), REGION)
        self.assertEqual(sum(c['dig'] == 1 for c in after['cells']), 4)
        self.assertEqual(sum(c['priority'] == 4000 for c in after['cells']), 4)
        self.assertEqual(sum(c['cooldown'] == 0 for c in after['cells']), 16)
        self.assertFalse(d.effect(VECTORS['designated'], value)['excavation_completion_proven'])

    def test_all_designated_bits_truncations_and_rehashed_forgeries_rejected(self):
        raw, value = VECTORS['designated'], intent()
        for index in range(len(raw)):
            for bit in range(8):
                bad = bytearray(raw); bad[index] ^= 1 << bit
                with self.assertRaises(d.Rejected):
                    d.effect(bytes(bad), value)
        for name in ('prepared', 'designated', 'cancelled'):
            for size in range(len(VECTORS[name])):
                with self.assertRaises(d.Rejected):
                    d.effect(VECTORS[name][:size], value)
            with self.assertRaises(d.Rejected):
                d.effect(VECTORS[name] + b'\0', value)
        for index in (8, 16, 24, 32, 52, 53, 85, 117, 135, 139, 140, 171, 206):
            bad = bytearray(raw); bad[index] ^= 1
            with self.assertRaises(d.Rejected):
                d.effect(rehash(bytes(bad)), value)

    def test_phase_reason_presence_table_and_canonical_absence(self):
        value = intent()
        for state in range(6):
            for reason in range(4):
                for known in range(3):
                    valid = (state in (0, 1, 2) and reason == 0 or state == 4 and reason in (1, 2)) and known == int(state == 2)
                    raw = outcome(state, reason, known)
                    if valid:
                        d.effect(raw, value)
                    else:
                        with self.assertRaises(d.Rejected):
                            d.effect(raw, value)
        for state in (0, 1, 4):
            for index in (139, 140, 171, 172):
                bad = bytearray(outcome(state, 1 if state == 4 else 0)); bad[index] ^= 1
                if state == 4 and index != 172:
                    bad = rehash(bytes(bad))
                with self.assertRaises(d.Rejected):
                    d.effect(bytes(bad), value)

    def test_complete_halo_bounds_and_redacted_cells(self):
        raw = VECTORS['observation']
        for size in range(len(raw)):
            with self.assertRaises(d.Rejected):
                d.observation(raw[:size], REGION)
        with self.assertRaises(d.Rejected):
            d.observation(raw + b'\0', REGION)
        for changes in ({'width': 9}, {'x': 0}, {'z': 32767}, {'height': True}):
            with self.assertRaises(d.Rejected):
                d.region({**REGION, **changes})
        original = d.observation(raw, REGION)
        first = original['cells'][0]['offset']
        hidden = raw[:first] + b'\1' + raw[first + 32:]
        o = d.observation(hidden, REGION)
        self.assertEqual(o['cells'][0], {'coordinate': [14, 14, 1], 'offset': first, 'presence': 1})
        self.assertEqual(d.blockers(o, True), [])
        self.assertIn('unacknowledged_hidden_context', d.blockers(o, False))
        target = next(c for c in original['cells'] if c['coordinate'] == [15, 15, 2])['offset']
        for presence in (0, 1):
            o = d.observation(raw[:target] + bytes([presence]) + raw[target + 32:], REGION)
            self.assertIn('unobserved_target', d.blockers(o, True))
        for hazards in (1, 2, 4, 8):
            bad = bytearray(raw); bad[first + 30] = hazards
            self.assertIn('known_visible_hazard', d.blockers(d.observation(bytes(bad), REGION), True))

    def test_all_geometry_plan_bindings_and_hidden_acknowledgement(self):
        witness = bytes(32)
        plans = set()
        for width in range(1, 9):
            for height in range(1, 9):
                for hidden in (False, True):
                    plans.add(d.plan_for({**REGION, 'width': width, 'height': height}, hidden, witness))
        self.assertEqual(len(plans), 128)
        for bad in ('', 'x y', 'a/b', '\u00e9', 'a' * 129):
            with self.assertRaises(d.Rejected):
                d.key_bytes(bad)
        for n in (1, 128):
            self.assertEqual(len(d.key_bytes('x' * n)), n + 2)
        wrong = intent(); wrong['allow_hidden_neighbors'] = True
        with self.assertRaises(d.Rejected):
            d.verify_intent(wrong)

    def test_minimal_proto_and_closed_fields(self):
        self.assertEqual(d.decode(d.encode({1: 1, 2: b'value', 11: 0})), {1: 1, 2: b'value', 11: 0})
        for raw in (b'\x08\x80\0', b'\x08\x01\x08\x01', b'\0\0', b'\x0b', b'\x08' + b'\xff' * 10,
                    b'\x12\x05ab', b'\x60\x00', b'\x80\x80\x80\x80\x80\x80\x80\x80\x80\x02'):
            with self.assertRaises(d.Rejected):
                d.decode(raw)
        self.assertEqual(d.decode(d.encode({1: 2**64 - 1}))[1], 2**64 - 1)

    def test_private_capsule_is_immutable_exact_and_locked(self):
        self.write(); original = self.path.read_bytes()
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)
        with d.capsule(self.path) as owner:
            self.assertEqual(owner.intent, intent()); owner.verify()
            with self.assertRaises(OSError):
                with d.capsule(self.path):
                    self.fail('duplicate custody acquired')
        with self.assertRaises(FileExistsError):
            self.write()
        self.assertEqual(self.path.read_bytes(), original)
        with d.capsule(self.path) as owner:
            raw = bytearray(original); raw[10] ^= 1; self.path.write_bytes(raw)
            with self.assertRaises(d.Rejected):
                owner.verify()
        with self.assertRaises((d.Rejected, ValueError)):
            with d.capsule(self.path):
                self.fail('corrupt bytes accepted')

    def test_symlinks_special_files_hardlinks_and_modes_refused(self):
        self.write(); self.path.rename(self.root / 'original.json')
        self.path.symlink_to(self.root / 'original.json')
        with self.assertRaises((OSError, d.Rejected)):
            with d.capsule(self.path):
                self.fail('symlink accepted')
        self.path.unlink(); os.link(self.root / 'original.json', self.path)
        with self.assertRaises(d.Rejected):
            with d.capsule(self.path):
                self.fail('hard link accepted')
        self.path.unlink(); os.mkfifo(self.path, 0o600)
        started = time.monotonic()
        with self.assertRaises(d.Rejected):
            with d.capsule(self.path):
                self.fail('FIFO accepted')
        self.assertLess(time.monotonic() - started, 1)
        self.path.unlink(); (self.root / 'original.json').rename(self.path)
        os.chmod(self.path, 0o640)
        with self.assertRaises(d.Rejected):
            with d.capsule(self.path):
                self.fail('permissive file accepted')
        os.chmod(self.path, 0o600); os.chmod(self.root, 0o750)
        with self.assertRaises(d.Rejected):
            with d.capsule(self.path):
                self.fail('permissive parent accepted')
        os.chmod(self.root, 0o700)
        alias = self.root / 'alias'; alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            with d.capsule(alias / 'intent.json'):
                self.fail('symlink parent accepted')

    def test_capsule_source_checksum_shape_and_extent_rejections(self):
        for change in ({'extra': 1}, {'effect_status': 'designated'}, {'prepare_token': '0' * 32},
                       {'witness': '0' * 64}, {'manifest': {**MANIFEST, 'generation': 8}},
                       {'endpoint': '127.0.0.1:05000'}):
            with self.assertRaises(d.Rejected):
                d.verify_intent({**intent(), **change})
        self.path.write_bytes(b''); os.chmod(self.path, 0o600)
        with self.assertRaises(d.Rejected):
            with d.capsule(self.path):
                self.fail('empty file repaired')
        self.assertEqual(self.path.read_bytes(), b'')
        self.path.write_bytes(b' ' * (d.MAX_CAPSULE + 1))
        with self.assertRaises(d.Rejected):
            with d.capsule(self.path):
                self.fail('oversized capsule accepted')
        with self.assertRaises(d.Rejected):
            json.loads('{"a":1,"a":2}', object_pairs_hook=d.unique_object)

    def test_fragmented_full_start_then_query_never_recommits(self):
        game = FakeGame()
        with Peer(game) as peer:
            with d.Client(peer.address, TOKEN, 3000) as client:
                result = self.start(client)
                self.assertEqual(result['effect_status'], 'designated')
                self.assertEqual(result['effect']['designated_count'], 4)
                with d.capsule(self.path) as owner:
                    query = d.recovered_result(client.query(owner.intent), False)
                    self.assertEqual(query['effect'], result['effect'])
        self.assertEqual(game.calls.count('CommitDesignation'), 1)
        self.assertEqual(game.calls, ['Handshake', 'ReadDesignation', 'PrepareDesignation', 'CommitDesignation', 'QueryDesignation'])

    def test_lost_commit_reply_recovers_by_new_query_only(self):
        game = FakeGame(); game.commit_mode = 'lost'
        with Peer(game, connections=2) as peer:
            with d.Client(peer.address, TOKEN, 3000) as client:
                with self.assertRaises(d.Rejected):
                    self.start(client)
                self.assertTrue(client.closed)
            self.assertTrue(self.path.exists()); self.assertEqual(game.calls.count('CommitDesignation'), 1)
            original = self.path.read_bytes()
            with d.capsule(self.path) as owner, d.Client(peer.address, TOKEN, 3000) as client:
                result = d.recovered_result(client.query(owner.intent), False)
                self.assertEqual(result['effect_status'], 'designated')
                owner.verify()
            self.assertEqual(self.path.read_bytes(), original)
        self.assertEqual(game.calls.count('CommitDesignation'), 1)
        self.assertEqual(game.calls[-2:], ['Handshake', 'QueryDesignation'])

    def test_native_unknown_and_forged_or_prepared_commit_never_retry(self):
        for index, mode in enumerate(('unknown', 'forged', 'prepared')):
            self.path = self.case_path(f'case-{index}')
            game = FakeGame(); game.commit_mode = mode
            with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                if mode == 'unknown':
                    self.assertEqual(self.start(client)['effect_status'], 'unknown')
                else:
                    with self.assertRaises(d.Rejected):
                        self.start(client)
                    self.assertTrue(client.closed)
                with self.assertRaises((d.Rejected, OSError)):
                    self.start(client)
            self.assertEqual(game.calls.count('CommitDesignation'), 1)

    def test_replayed_preparations_never_dispatch_even_with_new_capsule(self):
        for index, record in enumerate((VECTORS['prepared'], VECTORS['designated'], outcome(1), VECTORS['cancelled'])):
            self.path = self.case_path(f'case-{index}')
            game = FakeGame(); game.record = record; game.replayed = True
            with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                result = self.start(client)
                self.assertTrue(result['native_preparation_replayed']); self.assertFalse(result['commit_attempted_this_call'])
            self.assertNotIn('CommitDesignation', game.calls)

    def test_confirmation_or_changed_terrain_prevents_capsule_and_prepare(self):
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            value = intent(peer.address)
            with self.assertRaises(d.Rejected):
                d.start(client, self.path, 'dig-001', REGION, False, value['witness'], '0' * 64)
            self.assertEqual(game.calls, ['Handshake'])
            changed = bytearray(game.observation); struct.pack_into('>Q', changed, 24, 12346)
            game.observation = bytes(changed)
            with self.assertRaises(d.Rejected):
                self.start(client)
        self.assertFalse(self.path.exists()); self.assertNotIn('PrepareDesignation', game.calls)

    def test_independent_file_and_directory_sync_failures_prevent_prepare(self):
        real_sync = os.fsync
        for failing in (1, 2):
            self.path = self.case_path(f'sync-{failing}')
            game = FakeGame(); count = []
            def sync(fd):
                count.append(fd)
                if len(count) == failing:
                    raise OSError('injected sync failure')
                real_sync(fd)
            with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                with patch.object(d.os, 'fsync', side_effect=sync), self.assertRaises(OSError):
                    self.start(client)
            self.assertEqual(len(count), failing); self.assertTrue(self.path.exists())
            self.assertNotIn('PrepareDesignation', game.calls); self.assertNotIn('CommitDesignation', game.calls)

    def test_capsule_changed_between_stages_fences_dispatch_or_acknowledgement(self):
        for index, stage in enumerate(('prepare', 'commit')):
            self.path = self.case_path(f'substituted-{index}')
            game = FakeGame()
            def corrupt():
                raw = bytearray(self.path.read_bytes()); raw[10] ^= 1; self.path.write_bytes(raw)
            if stage == 'prepare':
                game.after_prepare = corrupt
            else:
                game.after_commit = corrupt
            with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                with self.assertRaises(d.Rejected):
                    self.start(client)
            self.assertEqual(game.calls.count('CommitDesignation'), int(stage == 'commit'))

    def test_absence_and_cancel_do_not_claim_undo(self):
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            value = intent(peer.address)
            missing = d.recovered_result(client.query(value), False)
            self.assertEqual(missing['effect_status'], 'unknown'); self.assertFalse(missing['absence_proves_non_application'])
            prepared = client.prepare(value)
            self.assertFalse(prepared['replayed'])
            cancelled = d.recovered_result(client.cancel(value), False)
            self.assertEqual(cancelled['effect']['reason'], 'cancelled_before_dispatch')
            game.record = outcome(1)
            self.assertEqual(d.recovered_result(client.cancel(value), False)['effect_status'], 'unknown')
        self.assertNotIn('CommitDesignation', game.calls)

    def test_bad_nonce_shape_bindings_or_incarnation_fence_connection(self):
        for fault in (('ReadDesignation', 3, b'n' * 32), ('ReadDesignation', 11, 0), ('ReadDesignation', 6, 8)):
            game = FakeGame(); game.fail = fault
            with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
                with self.assertRaises(d.Rejected):
                    client.observe(REGION)
                self.assertTrue(client.closed)
                with self.assertRaises(d.Rejected):
                    client.observe(REGION)
        game = FakeGame(); game.binding_alias = True
        with Peer(game) as peer, self.assertRaises(d.Rejected):
            d.Client(peer.address, TOKEN, 3000)
        self.assertEqual(game.calls, [])

    def test_notifications_deadlines_and_source_endpoint_binding(self):
        game = FakeGame(); game.notifications = [1] * 9
        with Peer(game) as peer, self.assertRaises(d.Rejected):
            d.Client(peer.address, TOKEN, 3000)
        game = FakeGame()
        with Peer(game) as peer, d.Client(peer.address, TOKEN, 3000) as client:
            with self.assertRaises(d.Rejected):
                client.query(intent())
            self.assertEqual(game.calls, ['Handshake'])
            client.deadline = time.monotonic() - 1
            with self.assertRaises(d.Rejected):
                client.observe(REGION)
            self.assertEqual(game.calls, ['Handshake'])
        for address in ('example.com:5000', '8.8.8.8:5000', '127.0.0.1:0', '127.0.0.1:05000'):
            with patch.object(d.socket, 'socket', side_effect=AssertionError('socket opened')), self.assertRaises(d.Rejected):
                d.Client(address, TOKEN, 1000)

    def test_operator_gates_and_offline_inspect_ignore_credentials(self):
        allowed = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': 's' * 32}
        with patch.dict(os.environ, allowed, clear=True):
            self.assertEqual(d.environment(False), ('127.0.0.1:5000', TOKEN))
            with self.assertRaises(d.Rejected):
                d.environment(True)
        for extra in ({'DFMCP_ADMITTED_BRIDGE_PROTOCOL': '1.16'}, {'DFMCP_DIG_ALLOW_DESIGNATE': '0'},
                      {'DFMCP_RUN_TOKEN': 's' * 32}, {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': 'true'}):
            with patch.dict(os.environ, {**allowed, **extra}, clear=True), self.assertRaises(d.Rejected):
                d.environment(False)
        self.write()
        out = io.StringIO()
        with (patch.dict(os.environ, {}, clear=True), patch.object(d, 'Client', side_effect=AssertionError('offline connection')),
              contextlib.redirect_stdout(out)):
            self.assertEqual(d.main(['inspect', '--record', str(self.path)]), 0)
        result = json.loads(out.getvalue()); self.assertEqual(result['native_calls'], 0)
        self.assertEqual(result['effect_status'], 'unknown'); self.assertNotIn('s' * 32, out.getvalue())

    def test_actual_cli_start_and_offline_inspection_are_structured(self):
        game = FakeGame()
        with Peer(game) as peer:
            value = intent(peer.address)
            env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
            env.update(DFMCP_ALLOW_UNADMITTED_DIG_V1_16='1', DFMCP_DIG_ALLOW_DESIGNATE='1',
                       DFMCP_DIG_TOKEN=TOKEN.decode(), DFMCP_DIG_ENDPOINT=peer.address)
            command = [sys.executable, str(ROOT / 'scripts/dig_designation_client.py'), 'start', '--record', str(self.path),
                       '--key', 'dig-001', '--expected-witness', value['witness'], '--confirm-plan', value['plan_digest']]
            for k in d.REGION_KEYS:
                command += ['--' + k, str(REGION[k])]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            parsed = json.loads(result.stdout); self.assertEqual(parsed['effect_status'], 'designated')
            self.assertNotIn(TOKEN.decode(), result.stdout)
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/dig_designation_client.py'), 'inspect', '--record', str(self.path)],
                                env={}, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        recovered = json.loads(result.stdout)
        self.assertEqual(recovered['effect_status'], 'designated')
        self.assertTrue(recovered['terminal_receipt_retained'])
        self.assertEqual(recovered['native_calls'], 0)
        self.assertFalse(recovered['excavation_completion_proven'])

    def test_maximal_capture_and_intent_fit_their_actual_serialized_bounds(self):
        selected = dict(zip(d.REGION_KEYS, (32759, 32759, 32766, 8, 8)))
        generation = 2**64 - 2
        raw = b'DFMDG016' + struct.pack('>QQQIIII', generation, 2**64 - 3, d.MAX_TICK,
                                        2**31 - 1, 32768, 32768, 32768)
        raw += d.region_bytes(selected) + b'\1' + d.text(b'\1' * 512) + struct.pack('>H', 300)
        cell = b'\2' + struct.pack('>IIIIIIHHBBB', 2**32 - 1, 2**32 - 1, 2**32 - 1,
                                   7000, 2**32 - 1, 2**32 - 1, 10080, 10080, 0, 0, 1)
        raw += cell * 300
        observed = d.observation(raw, selected)
        source = {'generation': generation, 'df_version': '\1' * 128, 'dfhack_version': '\1' * 128}
        response = d.canonical(d.observed_result({'observation': observed, 'manifest': source}, False))
        self.assertEqual(len(observed['cells']), 300)
        self.assertLessEqual(len(response), d.MAX_OUTPUT)
        value = d.build_intent('127.0.0.1:65535', 'k' * 128, selected, False, raw, source)
        with d.capsule(self.path, value) as owner:
            owner.verify()
            self.assertLessEqual(len(owner.raw), d.MAX_CAPSULE)
        self.assertEqual(d.observation(d.expected_after(raw, selected), selected)['sequence'], 2**64 - 2)

    def test_complete_response_is_bounded_and_failures_do_not_leak_secrets(self):
        result = d.observed_result({'observation': d.observation(VECTORS['observation'], REGION), 'manifest': MANIFEST}, False)
        self.assertLess(len(d.canonical(result)), d.MAX_OUTPUT)
        self.assertFalse(result['excavation_safety_proven'])
        bad = io.StringIO()
        with patch.dict(os.environ, {'DFMCP_DIG_TOKEN': 'private-secret'}, clear=True), contextlib.redirect_stdout(bad):
            status = d.main(['observe', '--x', '1', '--y', '1', '--z', '1', '--width', '1', '--height', '1'])
        self.assertEqual(status, 2); self.assertNotIn('private-secret', bad.getvalue())
        self.assertEqual(json.loads(bad.getvalue())['effect_status'], 'unknown')


def main() -> None:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(DigClientTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful():
        raise SystemExit(1)
    inputs = ['scripts/dig_designation_client.py', 'scripts/test_dig_designation_client.py',
              'scripts/test_dig_designation_native.py', 'bridge/common/dig_designation_v1_16.h']
    print(json.dumps({'schema': 'dfmcp.dig-client-evidence/1', 'status': 'passed_loopback_and_local_custody',
        'test_groups': result.testsRun, 'native_fixture_matches': 4,
        'designated_bit_corruptions_rejected': len(VECTORS['designated']) * 8,
        'effect_prefixes_rejected': sum(len(VECTORS[n]) for n in ('prepared', 'designated', 'cancelled')),
        'observation_prefixes_rejected': len(VECTORS['observation']), 'phase_reason_presence_cases': 72,
        'actual_cli_executed': True, 'actual_loopback_sockets': True, 'local_posix_custody_executed': True,
        'power_loss_campaign': False, 'real_dfhack_sdk_or_game': False, 'rust_or_mcp_executed': False,
        'source_sha256': {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in inputs}}, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
