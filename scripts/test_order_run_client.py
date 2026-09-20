#!/usr/bin/env python3
"""Execute developer codec, durable coordinator and supervised loopback RPC tests.

Also compile the real C++ engine/encoder and decode its emitted records. SDK,
real protobuf runtime, DFHack/live-game and Rust/MCP qualification are not implied.
Test journals and logs are retained, never deleted.
"""
from contextlib import redirect_stdout
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import order_run_client as c
import order_run_wire as w

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = Path(tempfile.mkdtemp(prefix='dfmcp-order-run-client-'))
SECRET = b'x' * 32
CAPTURE = b'DFMOR014' + struct.pack('>QQQIH', 41, 0, 100, 7, 4) + b'fort' + struct.pack('>BIIBBiiI', 1, 9, 10, 1, 1, 10, 10, 0)
PLAN = b'DFMOP014' + struct.pack('>IIBIIIH', 100, 1000, 1, 0, 1, 1, len(CAPTURE)) + CAPTURE


def reference_record(encoded_plan=PLAN, key='test', phase=0, reason=0, trigger=0, sample=None, count=0, observed=None, counted=100):
    name = key.encode(); name = struct.pack('>H', len(name)) + name
    digest = hashlib.sha256(b'dfmcp-order-run-plan/1\0' + encoded_plan).digest()
    token = hashlib.sha256(b'dfmcp-order-run-token/1\0' + name + digest).digest()[:16]
    payload = (b'DFMOE014' + name + struct.pack('>H', len(encoded_plan)) + encoded_plan + digest + token
               + bytes([phase, reason, trigger, int(phase in (1, 2, 3, 5)), int(phase == 3), int(observed is not None)])
               + struct.pack('>QIQH', observed or 0, count, counted, len(sample or b'')) + (sample or b''))
    return payload + hashlib.sha256(b'dfmcp-order-run-receipt/1\0' + payload).digest()


def sample(tick=101, status=1, sequence=1):
    data = bytearray(CAPTURE); data[16:24] = struct.pack('>Q', sequence); data[24:32] = struct.pack('>Q', tick)
    data[42] = 0  # paused byte follows site + length + "fort"
    data[-4:] = struct.pack('>I', status)
    return bytes(data)


def proto(fields):
    def varint(value):
        data = []
        while value >= 128:
            data.append(value % 128 + 128); value //= 128
        return bytes(data + [value])
    out = b''
    for number, value in fields.items():
        out += (varint(number * 8 + 2) + varint(len(value)) + value) if type(value) is bytes else (varint(number * 8) + varint(value))
    return out


def parse_request(data):
    # Independent small decoder in the server double, not the client codec.
    index, fields = 0, {}
    def varint():
        nonlocal index
        value, power = 0, 1
        while True:
            byte = data[index]; index += 1; value += (byte % 128) * power; power *= 128
            if byte < 128:
                return value
    while index < len(data):
        tag = varint(); field, kind = tag // 8, tag % 8
        if kind == 2:
            size = varint(); value = data[index:index + size]; index += size
        else:
            assert kind == 0; value = varint()
        assert field not in fields; fields[field] = value
    return fields


class NativeDouble:
    def __init__(self, lose=None, absent=False, invalid=None, delay=0):
        self.listener = socket.socket(); self.listener.bind(('127.0.0.1', 0)); self.listener.listen(4); self.listener.settimeout(.1)
        host, port = self.listener.getsockname(); self.endpoint = f'{host}:{port}'
        self.stop = threading.Event(); self.thread = threading.Thread(target=self.serve, daemon=False)
        self.lose, self.absent, self.invalid, self.delay = lose, absent, invalid, delay
        self.records, self.plans, self.calls, self.unpauses, self.error = {}, {}, [], 0, None

    @staticmethod
    def read(sock, length):
        out = b''
        while len(out) < length:
            part = sock.recv(length - len(out))
            if not part:
                raise EOFError()
            out += part
        return out

    def reply(self, sock, values):
        payload = proto(values); data = struct.pack('<h2xi', -1, len(payload)) + payload
        for i in range(0, len(data), 11):
            sock.sendall(data[i:i + 11])

    def connection(self, sock):
        sock.settimeout(2); sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        assert self.read(sock, 12) == b'DFHack?\n\x01\0\0\0'
        if self.delay:
            time.sleep(self.delay)
        sock.sendall(b'DFHack!\n\x01\0\0\0'); methods = {}
        while not self.stop.is_set():
            method, length = struct.unpack('<h2xi', self.read(sock, 8)); assert 0 <= length <= 2048
            fields = parse_request(self.read(sock, length))
            if method == 0:
                assert fields[2] == b'dfmcp.order_run.v1_14.Request' and fields[3] == b'dfmcp.order_run.v1_14.Reply'
                assert fields[4] == b'dfmcp_order_run_v1_14'; name = fields[1].decode(); assert name in w.METHODS
                bound = 2 if self.invalid == 'alias' else len(methods) + 2; methods[bound] = name
                if self.invalid == 'notifications':
                    sock.sendall(struct.pack('<h2xi', -3, 0) * 9)
                self.reply(sock, {1: bound}); continue
            name = methods[method]; self.calls.append(name)
            assert fields[1] == SECRET and fields[3] == 1 and fields[4] == 14
            reply = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 14, 6: 41, 7: b'test-df', 8: b'test-dfhack',
                     11: 0, 12: len(self.records)}
            if self.invalid == 'nonce': reply[3] = b'wrong-nonce'
            if self.invalid == 'version_type': reply[7] = 3
            if self.invalid == 'unknown_field': reply[13] = 0
            if name == 'Handshake':
                assert set(fields) == {1, 2, 3, 4}
            elif name == 'ObserveRun':
                assert set(fields) == {1, 2, 3, 4, 11} and fields[11] == 9; reply[9] = CAPTURE
            elif name == 'PrepareRun':
                assert set(fields) == {1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15}
                assert fields[8] == CAPTURE and fields[11] == 9
                encoded_plan = b'DFMOP014' + struct.pack('>IIBIIIH', fields[6], fields[7], fields[12], fields[13], fields[14], fields[15], len(CAPTURE)) + CAPTURE
                assert fields[9] == hashlib.sha256(b'dfmcp-order-run-plan/1\0' + encoded_plan).digest()
                key = fields[5].decode(); self.plans[key] = encoded_plan
                self.records[key] = reference_record(encoded_plan, key)
                reply[10] = self.records[key]; reply[12] = len(self.records)
            else:
                assert set(fields) == ({1, 2, 3, 4, 5, 9} if name == 'QueryRun' else {1, 2, 3, 4, 5, 9, 10})
                key = fields[5].decode(); encoded_plan = self.plans.get(key, PLAN)
                assert fields[9] == hashlib.sha256(b'dfmcp-order-run-plan/1\0' + encoded_plan).digest()
                if name != 'QueryRun':
                    expected = hashlib.sha256(b'dfmcp-order-run-token/1\0' + struct.pack('>H', len(key)) + key.encode() + fields[9]).digest()[:16]
                    assert fields[10] == expected
                if name == 'CommitRun':
                    self.unpauses += 1; self.records[key] = reference_record(encoded_plan, key, 1, observed=100)
                elif name == 'CancelRun':
                    self.records[key] = reference_record(encoded_plan, key, 3, 3, observed=101)
                if key in self.records and not self.absent:
                    reply[10] = self.records[key]; reply[12] = len(self.records)
                    decoded = w.record(self.records[key]); reply[11] = int(decoded['phase'] in ('running', 'stopping'))
            if name == self.lose:
                self.lose = None; sock.sendall(struct.pack('<h2xi', -1, 200)); return
            self.reply(sock, reply)

    def serve(self):
        try:
            while not self.stop.is_set():
                try:
                    sock, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with sock:
                    try:
                        self.connection(sock)
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        pass
        except BaseException as error:
            if not self.stop.is_set(): self.error = error
        finally:
            self.listener.close()

    def __enter__(self):
        self.thread.start(); return self

    def __exit__(self, *_):
        self.stop.set(); self.thread.join(3)
        if self.thread.is_alive():
            self.listener.close(); self.thread.join(3)
        if self.thread.is_alive():
            raise AssertionError('native double did not terminate')
        if self.error:
            raise self.error

    def client(self):
        return w.Client(self.endpoint, SECRET, 3000)


def private_path():
    directory = Path(tempfile.mkdtemp(dir=ARTIFACTS, prefix='journal-')); directory.chmod(0o700)
    return directory / 'runs.jsonl'


def source(endpoint='127.0.0.1:5000'):
    return dict(endpoint=endpoint, generation=41, df_version='test-df', dfhack_version='test-dfhack', folder='fort', site=7)


def entry(state='intent', proof=None, key='test'):
    return dict(key=key, plan=PLAN.hex(), state=state, native=proof.hex() if proof else None)


class WireTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.vectors = None
        compilers = [name for name in ('g++', 'clang++') if shutil.which(name)]
        if not compilers:
            raise RuntimeError('actual C++ encoder checks require an installed compiler')
        for name in compilers:
            binary = ARTIFACTS / (name + '-vectors')
            subprocess.run([name, '-std=c++17', '-Wall', '-Wextra', '-Werror', '-pedantic',
                            '-fsanitize=undefined', '-fno-sanitize-recover=all', '-I', str(ROOT),
                            str(ROOT / 'tests/native/order_run/client_vectors.cpp'), '-o', str(binary)], check=True, timeout=90)
            output = subprocess.run([str(binary)], check=True, text=True, capture_output=True, timeout=10).stdout
            (ARTIFACTS / (name + '-vectors.log')).write_text(output)
            vectors = dict(line.split('=') for line in output.splitlines())
            if cls.vectors is not None:
                assert vectors == cls.vectors, 'compiler vector outputs differ'
            cls.vectors = vectors

    def test_real_cpp_vectors_and_independent_python_encoding(self):
        self.assertEqual(len(self.vectors), 15)
        for value in self.vectors.values():
            w.record(bytes.fromhex(value))
        self.assertEqual(bytes.fromhex(self.vectors['0']), reference_record(key='vector'))
        self.assertTrue(w.record(bytes.fromhex(self.vectors['1']))['predicate_observed'])
        lost = w.record(bytes.fromhex(self.vectors['14']))
        self.assertTrue(lost['predicate_observed']); self.assertFalse(lost['pause_verified'])

    def test_every_byte_corruption_and_incomplete_prefix(self):
        raw = bytes.fromhex(self.vectors['1'])
        for i in range(len(raw)):
            bad = bytearray(raw); bad[i] ^= 1
            with self.assertRaises((ValueError, TypeError)):
                w.record(bytes(bad))
        for size in range(len(raw)):
            with self.assertRaises((ValueError, TypeError)):
                w.record(raw[:size])

    def test_plan_bounds_and_explicit_predicate_semantics(self):
        self.assertEqual(w.make_plan(CAPTURE, 100, 1000, 1), PLAN)
        for args in [(0, 1000, 1), (1201, 1000, 1), (100, 0, 1), (100, 60001, 1), (100, 1000, 4), (True, 1000, 1)]:
            with self.assertRaises(w.Rejected): w.make_plan(CAPTURE, *args)
        for threshold, count, interval in [(1, 1, 1), (0, 0, 1), (0, 17, 1), (0, 1, 0), (0, 16, 100)]:
            with self.assertRaises(w.Rejected): w.make_plan(CAPTURE, 100, 1000, 1, threshold, count, interval)
        changed = bytearray(CAPTURE); changed[-4:] = struct.pack('>I', 1)
        with self.assertRaises(w.Rejected): w.make_plan(bytes(changed), 100, 1000, 1)

    def test_semantically_forged_goal_and_identity_receipts(self):
        valid = reference_record(phase=3, reason=3, trigger=1, sample=sample(), count=1, observed=101, counted=101)
        self.assertTrue(w.record(valid)['predicate_observed'])
        for raw in [reference_record(phase=3, reason=3, trigger=1, sample=sample(status=0), count=1, observed=101, counted=101),
                    reference_record(phase=3, reason=3, trigger=1, sample=sample(tick=100), count=1, observed=100, counted=100),
                    reference_record(phase=3, reason=3, trigger=1, sample=sample(sequence=0), count=1, observed=101, counted=101),
                    reference_record(phase=3, reason=3, trigger=2, sample=sample(), observed=101),
                    reference_record(phase=3, reason=3, trigger=3, sample=sample(), observed=101),
                    reference_record(phase=1, reason=0, trigger=1, sample=sample(), count=1, observed=101, counted=101)]:
            with self.assertRaises(w.Rejected): w.record(raw)

    def test_boolean_state_matrix_rejects_rehashed_impossible_flags(self):
        for raw in self.vectors.values():
            data = bytes.fromhex(raw); r = w.Reader(data); r.take(8); r.field(128); r.field(604); r.take(48)
            position = r.offset; phase = data[position]
            for attempted in (0, 1):
                for verified in (0, 1):
                    changed = bytearray(data[:-32]); changed[position + 3] = attempted; changed[position + 4] = verified
                    changed = bytes(changed); changed += w.hashed(b'dfmcp-order-run-receipt/1', changed)
                    valid = attempted == int(phase in (1, 2, 3, 5)) and verified == int(phase == 3)
                    if valid: w.record(changed)
                    else:
                        with self.assertRaises(w.Rejected): w.record(changed)

    def test_maximum_record_and_whole_page_output_bounds(self):
        folder = b'\x01' * 512
        before = b'DFMOR014' + struct.pack('>QQQIH', 41, 0, 100, 7, len(folder)) + folder + struct.pack('>BIIBBiiI', 1, 9, 10, 1, 1, 10, 10, 0)
        encoded = w.make_plan(before, 1200, 60000, 1)
        last = bytearray(before); last[16:24] = struct.pack('>Q', 1); last[24:32] = struct.pack('>Q', 101)
        last[38 + len(folder)] = 0; last[-4:] = struct.pack('>I', 1)
        raw = reference_record(encoded, 'x' * 128, phase=3, reason=3, trigger=1,
                               sample=bytes(last), count=1, observed=101, counted=101)
        self.assertEqual(len(raw), 1425)
        stored = dict(key='x' * 128, plan=encoded.hex(), state='terminal', native=raw.hex())
        packet = c.canonical(dict(ok=True, profile='order-run/1.14', runtime_admitted=False,
                                  result=dict(records=[c.summary(stored)] * 8)))
        self.assertLess(len(packet), 128 * 1024)

    def test_wire_parser_bounds(self):
        for data in (b'\x08\x01\x08\x01', b'\x08\x80\x00', b'\x00', b'\x0a\xff', b'\x08' + b'\xff' * 10):
            with self.assertRaises(w.Rejected): w.decode(data)
        for target in ('192.0.2.1:5000', 'localhost:5000', '127.0.0.1:0', '127.0.0.1:05000'):
            with self.assertRaises(w.Rejected): w.address(target)


class JournalTests(unittest.TestCase):
    def test_synced_intent_prepare_and_offline_reopen(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            journal.append(entry()); journal.append(entry('prepared', reference_record())); head = journal.head
        with c.open_journal(path) as journal:
            self.assertEqual(journal.head, head); self.assertEqual(journal.entries['test']['state'], 'prepared')
            with self.assertRaises(w.Rejected): journal.append(entry('dispatch_started', reference_record()))

    def test_dispatch_marker_survives_reopen_and_blocks_new_intents(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            journal.append(entry()); journal.append(entry('prepared', reference_record())); journal.append(entry('dispatch_started', reference_record()))
        with c.open_journal(path, True) as journal:
            self.assertEqual(journal.entries['test']['state'], 'dispatch_started')
            with self.assertRaises(w.Rejected): journal.append(entry('prepared', reference_record()))
            with self.assertRaises(w.Rejected): journal.append(entry(key='another'))

    def test_complete_uncertain_sync_recovers_but_never_redispatches(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            journal.append(entry()); journal.append(entry('prepared', reference_record()))
            with patch.object(c.os, 'fsync', side_effect=OSError('uncertain sync')):
                with self.assertRaises(OSError): journal.append(entry('dispatch_started', reference_record()))
            self.assertTrue(journal.fenced)
        with c.open_journal(path, True) as journal:
            self.assertEqual(journal.entries['test']['state'], 'dispatch_started')

    def test_torn_tail_corruption_and_foreign_source_are_refused_unchanged(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            journal.append(entry()); original = journal.data
        for data in (original[:-1], original[:-7], original + b'{', original.replace(b'"site":7', b'"site":8')):
            path.write_bytes(data); path.chmod(0o600)
            with self.assertRaises((w.Rejected, ValueError)):
                with c.open_journal(path, True): pass
            self.assertEqual(path.read_bytes(), data)
        path.write_bytes(original)
        with self.assertRaises(w.Rejected):
            with c.open_journal(path, True, dict(source(), site=8)): pass

    def test_exact_byte_and_inode_custody_rechecked(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            data = bytearray(path.read_bytes()); data[10] ^= 1; path.write_bytes(data)
            with self.assertRaises(w.Rejected): journal.append(entry())
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            path.rename(path.with_suffix('.old')); path.write_bytes(journal.data); path.chmod(0o600)
            with self.assertRaises(w.Rejected): journal.verify()

    def test_links_modes_special_files_and_locking(self):
        path = private_path()
        with c.open_journal(path, True, source()):
            with self.assertRaises(OSError):
                with c.open_journal(path, True): pass
        linked = path.with_suffix('.link'); linked.symlink_to(path)
        with self.assertRaises(OSError):
            with c.open_journal(linked): pass
        hard = path.with_suffix('.hard'); os.link(path, hard)
        with self.assertRaises(w.Rejected):
            with c.open_journal(path): pass
        fifo = private_path(); os.mkfifo(fifo, 0o600)
        with self.assertRaises(w.Rejected):
            with c.open_journal(fifo): pass
        path = private_path(); path.write_text('x'); path.chmod(0o644)
        with self.assertRaises(w.Rejected):
            with c.open_journal(path): pass

    def test_partial_dispatch_write_and_rehashed_illegal_history(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            journal.append(entry()); journal.append(entry('prepared', reference_record()))
            before = journal.data
            write = os.write
            def partial(fd, data):
                write(fd, bytes(data[:5]))
                raise OSError('partial write')
            with patch.object(c.os, 'write', side_effect=partial):
                with self.assertRaises(OSError): journal.append(entry('dispatch_started', reference_record()))
        partial_bytes = path.read_bytes()
        with self.assertRaises(ValueError):
            with c.open_journal(path, True): pass
        self.assertEqual(path.read_bytes(), partial_bytes)
        # Valid checksums cannot make a dispatch->prepared reversal legal.
        header, head = c.unseal(before.splitlines(keepends=True)[0]); data = c.sealed(header)
        for number, state in enumerate(('intent', 'prepared', 'dispatch_started', 'prepared'), 1):
            frame = c.sealed(dict(sequence=number, previous=head,
                                 entry=entry(state, None if state == 'intent' else reference_record())))
            _, head = c.unseal(frame); data += frame
        path.write_bytes(data)
        with self.assertRaises(w.Rejected):
            with c.open_journal(path, True): pass
        self.assertEqual(path.read_bytes(), data)

    def test_all_state_pairs_and_frozen_terminal_evidence(self):
        allowed = {'intent': {'prepared','tracking','terminal','cancelled_before_dispatch'},
                   'prepared': {'dispatch_started','tracking','terminal','cancelled_before_dispatch'},
                   'dispatch_started': {'tracking','terminal','cancel_requested'},
                   'tracking': {'tracking','terminal','cancel_requested'},
                   'cancel_requested': {'tracking','terminal','cancel_requested'},
                   'terminal': set(), 'cancelled_before_dispatch': set()}
        prepared = reference_record(); running = reference_record(phase=1, observed=100)
        stopped = reference_record(phase=3, reason=1, observed=200)
        for old in c.STATES:
            previous = entry(old, None if old == 'intent' else prepared if old in ('prepared','dispatch_started','cancelled_before_dispatch') else stopped if old == 'terminal' else running)
            for new in c.STATES:
                raw = stopped if new == 'terminal' else prepared if new == 'prepared' else running if new == 'tracking' else None
                updated = entry(new, raw)
                if new in ('dispatch_started','cancel_requested','cancelled_before_dispatch'):
                    updated['native'] = previous['native']
                if new in allowed[old]: c.transition(previous, updated, source())
                else:
                    with self.assertRaises(w.Rejected): c.transition(previous, updated, source())

    def test_capacity_keeps_terminal_slot_and_never_evicts(self):
        path = private_path()
        with c.open_journal(path, True, source()) as journal:
            journal.append(entry()); journal.append(entry('prepared', reference_record())); journal.append(entry('dispatch_started', reference_record()))
            with patch.object(c, 'MAX_EVENTS', 4):
                with self.assertRaises(w.Rejected): journal.append(entry('tracking', reference_record(phase=1, observed=100)))
                journal.append(entry('terminal', reference_record(phase=3, reason=1, observed=200)))
            self.assertEqual(journal.entries['test']['state'], 'terminal')


class FlowTests(unittest.TestCase):
    def initialize(self, server, path):
        with server.client() as client: return c.prepare(client, path, 'test', PLAN)

    def test_prepare_confirm_commit_query_cancel_and_offline_evidence(self):
        path = private_path()
        with NativeDouble() as server:
            result = self.initialize(server, path); self.assertEqual(result['state'], 'prepared'); self.assertEqual(server.unpauses, 0)
            with c.open_journal(path, True) as journal, server.client() as client:
                result = c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
                self.assertEqual(result['state'], 'tracking')
                with self.assertRaises(w.Rejected): c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
            with c.open_journal(path, True) as journal, server.client() as client:
                result = c.operate(journal, client, 'cancel', 'test'); self.assertTrue(result['native_evidence']['pause_verified'])
            self.assertEqual(server.unpauses, 1)
        with patch.dict(os.environ, {}, clear=True), patch.object(w, 'Client', side_effect=AssertionError('offline socket')):
            output = io.StringIO()
            with redirect_stdout(output): code = c.main(['inspect', '--journal', str(path), '--key', 'test'])
            self.assertEqual(code, 0); self.assertTrue(json.loads(output.getvalue())['result']['native_evidence']['current_pause_unproved'])

    def test_lost_commit_reply_reopens_only_for_receipt_query(self):
        path = private_path()
        with NativeDouble(lose='CommitRun') as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                with self.assertRaises(w.Rejected): c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
            with c.open_journal(path, True) as journal, server.client() as client:
                self.assertEqual(journal.entries['test']['state'], 'dispatch_started')
                result = c.operate(journal, client, 'query', 'test'); self.assertEqual(result['state'], 'tracking')
                with self.assertRaises(w.Rejected): c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
            self.assertEqual(server.unpauses, 1); self.assertEqual(server.calls.count('CommitRun'), 1)

    def test_lost_prepare_reply_can_be_reconciled_without_repreparation(self):
        path = private_path()
        with NativeDouble(lose='PrepareRun') as server:
            with self.assertRaises(w.Rejected): self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                self.assertEqual(journal.entries['test']['state'], 'intent')
                result = c.operate(journal, client, 'query', 'test'); self.assertEqual(result['state'], 'prepared')
            self.assertEqual(server.calls.count('PrepareRun'), 1); self.assertEqual(server.unpauses, 0)

    def test_absent_native_record_remains_unknown_without_erasing_evidence(self):
        path = private_path()
        with NativeDouble() as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex()); head = journal.head
                server.absent = True
                result = c.operate(journal, client, 'query', 'test'); self.assertTrue(result['native_record_absent'])
                self.assertEqual(journal.head, head); self.assertTrue(result['unresolved'])
                with self.assertRaises(w.Rejected): journal.append(entry(key='new'))

    def test_file_and_directory_sync_failures_precede_native_prepare(self):
        for fail_call in (1, 2):
            path = private_path(); sync = os.fsync; calls = []
            def failing(fd):
                calls.append(fd)
                if len(calls) == fail_call: raise OSError('injected sync')
                return sync(fd)
            with NativeDouble() as server, server.client() as client, patch.object(c.os, 'fsync', side_effect=failing):
                with self.assertRaises(OSError): c.prepare(client, path, 'test', PLAN)
                self.assertNotIn('PrepareRun', server.calls); self.assertEqual(server.unpauses, 0)

    def test_dispatch_sync_failure_prevents_native_unpause(self):
        path = private_path()
        with NativeDouble() as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client, patch.object(c.os, 'fsync', side_effect=OSError('sync')):
                with self.assertRaises(OSError): c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
            self.assertNotIn('CommitRun', server.calls); self.assertEqual(server.unpauses, 0)

    def test_predicate_receipt_survives_restart_without_claiming_goods_or_current_pause(self):
        path = private_path()
        with NativeDouble() as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
            # Models a native callback's result between foreground requests;
            # Query itself neither samples the predicate nor advances the game.
            server.records['test'] = reference_record(phase=3, reason=3, trigger=1,
                sample=sample(), count=1, observed=101, counted=101)
            with c.open_journal(path, True) as journal, server.client() as client:
                result = c.operate(journal, client, 'query', 'test')
                self.assertEqual(result['state'], 'terminal')
        with c.open_journal(path) as journal:
            proof = c.summary(journal.get('test'))['native_evidence']
            self.assertTrue(proof['predicate_observed']); self.assertTrue(proof['pause_verified'])
            self.assertFalse(proof['goods_produced_proven']); self.assertTrue(proof['current_pause_unproved'])

    def test_source_loss_remains_blocking_and_prepared_receipt_cannot_undo_dispatch(self):
        path = private_path()
        with NativeDouble() as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                c.operate(journal, client, 'commit', 'test', w.plan_digest(PLAN).hex())
                server.records['test'] = reference_record()
                with self.assertRaises(w.Rejected): c.operate(journal, client, 'query', 'test')
                server.records['test'] = reference_record(phase=5, reason=7, trigger=6)
                result = c.operate(journal, client, 'query', 'test')
                self.assertTrue(result['unresolved']); self.assertFalse(result['native_evidence']['pause_verified'])
                with self.assertRaises(w.Rejected): journal.append(entry(key='new'))

    def test_confirmation_and_explicit_fortress_mismatch_have_no_effect(self):
        path = private_path()
        with NativeDouble() as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                with self.assertRaises(w.Rejected): c.operate(journal, client, 'commit', 'test', '0' * 64)
            env = {'DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14': '1', 'DFMCP_ORDER_RUN_ALLOW_CLOCK': '1',
                   'DFMCP_ORDER_RUN_ENDPOINT': server.endpoint, 'DFMCP_ORDER_RUN_TOKEN': SECRET.decode()}
            output = io.StringIO()
            with patch.dict(os.environ, env, clear=True), redirect_stdout(output):
                code = c.main(['prepare', '--journal', str(private_path()), '--key', 'test', '--order-id', '9',
                               '--world-folder', 'wrong-fort', '--site-id', '7', '--predicate', 'approved', '--ticks', '100', '--wall-ms', '1000'])
            self.assertEqual(code, 2); self.assertEqual(server.unpauses, 0)
            self.assertEqual(server.calls.count('PrepareRun'), 1)

    def test_native_protocol_malformations_and_deadlines(self):
        for kind in ('alias', 'notifications', 'nonce', 'version_type', 'unknown_field'):
            with NativeDouble(invalid=kind) as server:
                with self.assertRaises(w.Rejected): server.client()
                self.assertEqual(server.unpauses, 0)
        with NativeDouble(delay=.05) as server:
            with self.assertRaises((w.Rejected, OSError)): w.Client(server.endpoint, SECRET, 2)

    def test_query_has_no_hidden_poll_or_mutation_and_local_cancel_has_no_native_call(self):
        path = private_path()
        with NativeDouble() as server:
            self.initialize(server, path)
            with c.open_journal(path, True) as journal, server.client() as client:
                before = len(server.calls); c.operate(journal, client, 'query', 'test')
                self.assertEqual(server.calls[before:], ['QueryRun'])
                before = len(server.calls); result = c.operate(journal, client, 'cancel', 'test')
                self.assertEqual(server.calls[before:], []); self.assertEqual(result['state'], 'cancelled_before_dispatch')

    def test_environment_isolation(self):
        for extra in ({}, {'DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14':'1'},
                      {'DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14':'1','DFMCP_ORDER_RUN_ALLOW_CLOCK':'true'},
                      {'DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14':'1','DFMCP_ORDER_RUN_ALLOW_CLOCK':'1','DFMCP_ADMITTED_BRIDGE_PROTOCOL':'1.0'}):
            with patch.dict(os.environ, extra, clear=True):
                with self.assertRaises(w.Rejected): c.environment(True)


if __name__ == '__main__':
    print(f'Retained artifacts: {ARTIFACTS}', flush=True)
    unittest.main(verbosity=2)
