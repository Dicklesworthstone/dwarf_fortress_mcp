"""Real selected-file custody and joined run/1.13 TCP peers, never a game.

No Batch, map/inventory acquisition or placement owner is injected as if present.
Wire framing and native records are encoded independently of production codecs.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as storage
import route_furniture_clock_context as context
from route_furniture_review import Source
import recover_route_furniture_clock as recovery

BATCH = 'a' * 64
TOKEN = 'r' * 32
METHODS = ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun', 'CancelRun')


def env(address=('127.0.0.1', 5000)):
    value = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    value.update(DFMCP_ALLOW_UNADMITTED_RUN_V1_13='1', DFMCP_RUN_TOKEN=TOKEN,
                 DFMCP_RUN_ENDPOINT=f'{address[0]}:{address[1]}')
    return value


def scope_for(address=('127.0.0.1', 5000), final=False):
    source = Source(f'{address[0]}:{address[1]}', 'df', 'dfhack', 7, 9,
                    '0' * 64, '1' * 64, 100, (128, 128, 8), (0, 0, 0), (7, 4, 1),
                    (100, 200, 300))
    values = dict(schema=context.SCHEMA, policy=context.POLICY,
                  batch_id=BATCH, review_digest='b' * 64, plan_digest='c' * 64,
                  inventory_digest='d' * 64, next_step='chair', frontier_evidence='e' * 64,
                  source=source.json(), game_ticks=120, wall_ms=5000,
                  clock_manifest={'generation': 11, 'df_version': 'df', 'dfhack_version': 'dfhack'},
                  clock_observation_hex=(b'DFMRO013' + struct.pack('>QQQBBB', 11, 3, 100, 1, 1, 1)).hex())
    if final:
        values.update(schema=context.FINAL_SCHEMA, policy=context.FINAL_POLICY, phase='final_construction')
        values.pop('next_step')
    return (context.FinalScope if final else context.Scope).create(values)


def private_write(path, data):
    path.write_bytes(data)
    path.chmod(0o600)


class Fixture:
    def __init__(self, root, address=('127.0.0.1', 5000), *, final=False):
        self.root = Path(root)
        self.root.chmod(0o700)
        self.scope = scope_for(address, final)
        self.intent = self.scope.intent()
        self.path = recovery.record_path(str(self.root), self.scope.digest)
        self.sidecar = Path(str(self.path) + context.SCOPE_SUFFIX)
        self.outcome = Path(str(self.path) + storage.SUFFIX)
        private_write(self.path, clock.canonical({'intent': self.intent,
            'sha256': hashlib.sha256(clock.canonical(self.intent)).hexdigest()}) + b'\n')
        private_write(self.sidecar, self.scope.raw + b'\n')

    def owner(self, **kwargs):
        return recovery.Recovery(str(self.root), kwargs.get('batch_id', BATCH),
                                 kwargs.get('confirmation', self.scope.digest),
                                 kwargs.get('budget', recovery.Budget(10000)))

    def snapshot(self):
        return {p.name: p.read_bytes() for p in self.root.iterdir() if p.is_file()}

    def args(self, op, *extra):
        return [op, '--clock-directory', str(self.root), '--batch-id', BATCH,
                '--confirm-progress', self.scope.digest, *extra]

    def saved(self, phase='stopped', generation=11):
        result = {'manifest': {'generation': generation, 'df_version': 'df', 'dfhack_version': 'dfhack'},
                  'record': clock.decode_record(record(self.intent, phase))}
        with storage.OutcomeStore(self.path) as owner:
            owner.retain(result)


def record(intent, phase, *, reason=None):
    phases = {'prepared': 0, 'running': 1, 'stopping': 2, 'stopped': 3, 'refused': 4, 'source_lost': 5}
    phase_code = phases[phase]
    reason = {'prepared': 0, 'running': 0, 'stopping': 1, 'stopped': 1,
              'refused': 3, 'source_lost': 7}[phase] if reason is None else reason
    key = intent['idempotency_key'].encode('ascii')
    observation = bytes.fromhex(intent['observation_hex'])
    plan = hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + struct.pack('>II',
        intent['game_ticks'], intent['wall_ms']) + observation).digest()
    key_field = struct.pack('>H', len(key)) + key
    token = hashlib.sha256(b'dfmcp-bounded-run-token/1\0' + key_field + plan).digest()[:16]
    raw = b'DFMRE013' + key_field + struct.pack('>II', intent['game_ticks'], intent['wall_ms'])
    raw += observation + plan + token
    known = phase in ('running', 'stopping', 'stopped')
    raw += struct.pack('>BBBBBQ', phase_code, reason, phase in ('running', 'stopping', 'stopped', 'source_lost'),
                       phase == 'stopped', known, 220 if known else 0)
    return raw + hashlib.sha256(b'dfmcp-bounded-run-receipt/1\0' + raw).digest()


def encode(values):
    def varint(n):
        out = bytearray()
        while n > 127:
            out.append((n & 127) | 128)
            n >>= 7
        return bytes(out + bytes([n]))
    out = bytearray()
    for k, v in sorted(values.items()):
        if isinstance(v, bytes):
            out += varint(k * 8 + 2) + varint(len(v)) + v
        else:
            out += varint(k * 8) + varint(v)
    return bytes(out)


def decode(raw):
    offset, out = 0, {}
    def varint():
        nonlocal offset
        result = 0
        for i in range(10):
            b = raw[offset]
            offset += 1
            result |= (b & 127) << (7 * i)
            if b < 128:
                return result
        raise AssertionError('invalid test request varint')
    while offset < len(raw):
        tag = varint()
        assert tag >> 3 not in out
        if tag & 7 == 2:
            size = varint()
            v = raw[offset:offset + size]
            offset += size
        else:
            assert tag & 7 == 0
            v = varint()
        out[tag >> 3] = v
    assert offset == len(raw)
    return out


class Peer:
    """Exactly one joined connection, with independent frame/field assertions."""
    def __init__(self, phases=('stopped',), *, generation=11, change=None):
        self.phases, self.generation, self.change = phases, generation, change
        self.calls, self.errors, self.intent = [], [], None
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.address = self.listener.getsockname()
        self.listener.listen(1)
        self.listener.settimeout(5)
        self.connection = None

    def __enter__(self):
        self.thread = threading.Thread(target=self.serve)
        self.thread.start()
        return self

    def serve(self):
        def exact(conn, n):
            value = bytearray()
            while len(value) < n:
                piece = conn.recv(n - len(value))
                if not piece:
                    raise EOFError
                value += piece
            return bytes(value)
        try:
            conn, _ = self.listener.accept()
            self.connection = conn
            with conn:
                conn.settimeout(5)
                assert exact(conn, 12) == b'DFHack?\n' + struct.pack('<i', 1)
                conn.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                for i, name in enumerate(METHODS):
                    method, length = struct.unpack('<h2xi', exact(conn, 8))
                    values = decode(exact(conn, length))
                    assert method == 0 and values == {1: name.encode(), 2: b'dfmcp.run.v1_13.Request',
                        3: b'dfmcp.run.v1_13.Reply', 4: b'dfmcp_run_v1_13'}
                    self.reply(conn, {1: i + 2})
                while True:
                    method, length = struct.unpack('<h2xi', exact(conn, 8))
                    assert 0 <= length <= 2048
                    fields = decode(exact(conn, length))
                    name = METHODS[method - 2]
                    self.calls.append(name)
                    assert fields[1] == TOKEN.encode() and len(fields[2]) == 32 and fields[3] == 1 and fields[4] == 13
                    assert name in ('Handshake', 'QueryRun'), 'unexpected native mutation: ' + name
                    if name == 'Handshake':
                        assert set(fields) == {1, 2, 3, 4}
                        phase = None
                    else:
                        assert {k: v for k, v in fields.items() if k > 4} == {
                            5: self.intent['idempotency_key'].encode(), 9: bytes.fromhex(self.intent['plan_digest_hex'])}
                        index = self.calls.count('QueryRun') - 1
                        phase = self.phases[min(index, len(self.phases) - 1)]
                        if phase == 'drop':
                            return
                    values = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 13, 6: self.generation,
                              7: b'df', 8: b'dfhack', 11: int(phase in ('running', 'stopping')), 12: 1}
                    if phase is not None:
                        values[10] = record(self.intent, phase)
                    if self.change:
                        self.change(name, values)
                    self.reply(conn, values)
        except (EOFError, ConnectionResetError, BrokenPipeError):
            pass
        except BaseException as exc:
            self.errors.append(exc)

    @staticmethod
    def reply(conn, values):
        raw = encode(values)
        conn.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)

    def __exit__(self, *_):
        if self.connection is not None:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.listener.close()
        self.thread.join(6)
        assert not self.thread.is_alive(), 'test peer did not join'
        if self.errors:
            raise self.errors[0]


class RecoveryFileTests(unittest.TestCase):
    def test_both_scope_families_inspect_without_batch_or_credentials(self):
        for final in (False, True):
            with tempfile.TemporaryDirectory() as directory:
                f = Fixture(directory, final=final)
                before = f.snapshot()
                with patch.dict(os.environ, {}, clear=True), patch.object(socket, 'socket', side_effect=AssertionError('network')):
                    with f.owner() as owner:
                        out = owner.inspect()
                self.assertEqual(out['scope'], f.scope.value())
                self.assertFalse(out['original_batch_custody_verified'])
                self.assertFalse(out['clock_start_dispatched'])
                self.assertTrue(out['recovery_required'])
                self.assertEqual(before, f.snapshot())

    def test_resolved_receipts_stay_offline_and_never_prove_current_pause(self):
        for phase in ('stopped', 'refused', 'source_lost'):
            with tempfile.TemporaryDirectory() as directory:
                f = Fixture(directory, final=True)
                f.saved(phase)
                before = f.snapshot()
                with patch.dict(os.environ, {}, clear=True), patch.object(socket, 'socket', side_effect=AssertionError('network')):
                    with f.owner() as owner:
                        out = owner.query(1000)
                self.assertEqual(out['clock_result']['record']['phase'], phase)
                self.assertTrue(out['current_pause_unproved'])
                self.assertFalse(out['construction_completion_proven'])
                self.assertEqual(out['recovery_required'], phase == 'source_lost')
                self.assertFalse(out['clock_result']['native_contacted'])
                self.assertEqual(before, f.snapshot())

    def test_scope_batch_confirmation_and_intent_are_independently_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            with self.assertRaises(ValueError), f.owner(batch_id='f' * 64):
                pass
            other = 'f' * 64
            target = recovery.record_path(directory, other)
            private_write(target, f.path.read_bytes())
            private_write(Path(str(target) + context.SCOPE_SUFFIX), f.sidecar.read_bytes())
            with self.assertRaises(ValueError), f.owner(confirmation=other):
                pass
            changed = dict(f.intent, idempotency_key='another-clock-effect')
            changed['prepare_token_hex'] = clock.token_for(changed['idempotency_key'], bytes.fromhex(changed['plan_digest_hex'])).hex()
            private_write(f.path, clock.canonical({'intent': changed, 'sha256': hashlib.sha256(clock.canonical(changed)).hexdigest()}) + b'\n')
            with self.assertRaises(ValueError), f.owner():
                pass

    def test_torn_missing_foreign_or_modified_sidecars_are_not_adopted(self):
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            valid = f.sidecar.read_bytes()
            bad = [b'', valid[:-1], valid[:-2] + b'X\n', b'{}\n', valid + b'\n']
            for raw in bad:
                private_write(f.sidecar, raw)
                with self.subTest(raw=raw[:15]), self.assertRaises(ValueError), f.owner():
                    pass
                self.assertEqual(f.sidecar.read_bytes(), raw)
            f.sidecar.unlink()
            with self.assertRaises(OSError), f.owner():
                pass
            self.assertFalse(f.sidecar.exists())

    def test_insecure_or_replaced_files_and_competing_owner_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            f.sidecar.chmod(0o644)
            with self.assertRaises(ValueError), f.owner():
                pass
            f.sidecar.chmod(0o600)
            with self.assertRaises(ValueError), f.owner() as owner:
                with self.assertRaises(BlockingIOError), f.owner():
                    pass
                original = f.sidecar.read_bytes()
                f.sidecar.rename(f.root / 'replaced')
                private_write(f.sidecar, original)
                with self.assertRaises(ValueError):
                    owner.check()
                self.assertTrue(owner.fenced)
                # Catching the refusal does not revive this owner.
                with self.assertRaises(ValueError):
                    owner.inspect()
                # The normal context exit must still refuse a fenced owner.

    def test_symlink_hardlink_and_corrupt_outcome_refused_before_contact(self):
        for mode in ('symlink', 'hardlink', 'outcome'):
            with tempfile.TemporaryDirectory() as directory:
                f = Fixture(directory)
                if mode == 'outcome':
                    private_write(f.outcome, b'{bad\n')
                else:
                    target = f.root / 'other'
                    f.sidecar.rename(target)
                    (f.sidecar.symlink_to(target) if mode == 'symlink' else os.link(target, f.sidecar))
                with patch.object(socket, 'socket', side_effect=AssertionError('network')):
                    with self.assertRaises((OSError, ValueError)), f.owner():
                        pass

    def test_clock_authority_is_only_loaded_for_online_query(self):
        for extra in ('DFMCP_BUILD_TOKEN', 'DFMCP_ROUTE_FURNITURE_ALLOW_PLACE', 'DFMCP_ADMITTED_BRIDGE_PROTOCOL'):
            with patch.dict(os.environ, dict(env(), **{extra: '1'}), clear=True):
                with self.assertRaises(ValueError):
                    recovery.Authority.load()
        with patch.dict(os.environ, env(), clear=True):
            authority = recovery.Authority.load()
            self.assertNotIn(TOKEN, repr(authority))
            os.environ['DFMCP_RUN_TOKEN'] = 'z' * 32
            with self.assertRaises(ValueError):
                authority.clock_guard()

    def test_path_and_budget_bounds_apply_before_file_or_socket_access(self):
        for path in ('relative', '/private/../clock', '/private//clock', '/private/clock/', '/private/\0clock'):
            with self.assertRaises(ValueError):
                recovery.record_path(path, 'a' * 64)
        for ms in (0, 60001, True):
            with self.assertRaises(ValueError):
                recovery.Budget(ms)
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            b = recovery.Budget(); b.disk_bytes = 0
            with self.assertRaises(ValueError), f.owner(budget=b):
                pass
            b = recovery.Budget(); b.deadline += 1
            with self.assertRaises(ValueError), f.owner(budget=b):
                pass

    def test_cli_is_offline_single_output_and_has_no_batch_path_argument(self):
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory, final=True)
            before = f.snapshot()
            command = [sys.executable, '-B', str(Path(recovery.__file__)), *f.args('inspect')]
            result = subprocess.run(command, env={k: v for k, v in env().items() if not k.startswith('DFMCP_')},
                                    capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(result.stdout.splitlines()), 1)
            self.assertFalse(json.loads(result.stdout)['original_batch_custody_verified'])
            self.assertEqual(before, f.snapshot())
            for extra in (['--directory', '/missing/batch'], ['--wait-ms', '0'], ['--ticks', '100']):
                output = io.BytesIO()
                with patch.object(sys, 'stdout', types.SimpleNamespace(buffer=output)):
                    self.assertEqual(recovery.main(f.args('inspect', *extra)), 2)
                self.assertFalse(json.loads(output.getvalue())['ok'])
                self.assertEqual(before, f.snapshot())

    def test_import_and_output_do_not_require_or_lock_a_furniture_batch(self):
        scripts = str(Path(recovery.__file__).parent)
        result = subprocess.run([sys.executable, '-B', '-c',
            "import recover_route_furniture_clock,sys; "
            "assert 'route_furniture_batch' not in sys.modules; "
            "assert 'route_furniture_batch_store' not in sys.modules"],
            env=dict(env(), PYTHONPATH=scripts), capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            output = io.BytesIO()
            test = self
            class Sink:
                def write(self, raw):
                    with test.assertRaises(BlockingIOError), f.owner():
                        pass
                    return output.write(raw)
                def flush(self): pass
            with patch.object(sys, 'stdout', types.SimpleNamespace(buffer=Sink())):
                self.assertEqual(recovery.main(f.args('inspect')), 0)
            self.assertTrue(json.loads(output.getvalue())['ok'])

    def test_short_output_does_not_retry_or_emit_second_object(self):
        class Sink:
            def __init__(self): self.calls = 0
            def write(self, raw): self.calls += 1; return len(raw) - 1
            def flush(self): raise AssertionError('short write must not flush')
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            sink = Sink()
            with patch.object(sys, 'stdout', types.SimpleNamespace(buffer=sink)):
                self.assertEqual(recovery.main(f.args('inspect')), 2)
            self.assertEqual(sink.calls, 1)


class RecoveryTCPTests(unittest.TestCase):
    def test_original_query_persists_exact_terminal_receipt_and_reopens_offline(self):
        for final in (False, True):
            with tempfile.TemporaryDirectory() as directory, Peer() as peer:
                f = Fixture(directory, peer.address, final=final); peer.intent = f.intent
                before = f.snapshot()
                with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                    out = owner.query()
                self.assertEqual(peer.calls, ['Handshake', 'QueryRun'])
                self.assertTrue(out['clock_result']['storage_acknowledged_this_call'])
                self.assertFalse(out['recovery_required'])
                self.assertTrue(out['current_pause_unproved'])
                self.assertFalse(out['readiness_checked'])
                self.assertEqual({n: f.snapshot()[n] for n in before}, before)
                self.assertEqual(bytes.fromhex(json.loads(f.outcome.read_bytes())['outcome']['record_hex']), record(f.intent, 'stopped'))
                with patch.dict(os.environ, {}, clear=True), f.owner() as owner:
                    self.assertFalse(owner.query()['clock_result']['native_contacted'])

    def test_native_absence_prepared_and_unfinished_phases_remain_pending(self):
        for phase in (None, 'prepared', 'running', 'stopping'):
            with tempfile.TemporaryDirectory() as directory, Peer((phase,)) as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                before = f.snapshot()
                with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                    out = owner.query()
                self.assertTrue(out['recovery_required'])
                self.assertFalse(out['clock_result']['terminal_record_retained'])
                self.assertEqual(before, f.snapshot())
                self.assertEqual(peer.calls, ['Handshake', 'QueryRun'])

    def test_wait_polls_only_original_query_on_one_connection_and_budget(self):
        with tempfile.TemporaryDirectory() as directory, Peer(('running', 'stopped')) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            budget = recovery.Budget(); deadline = budget.deadline
            with patch.dict(os.environ, env(peer.address), clear=True), f.owner(budget=budget) as owner:
                out = owner.query(1000)
            self.assertEqual(peer.calls, ['Handshake', 'QueryRun', 'QueryRun'])
            self.assertEqual(budget.deadline, deadline)
            self.assertEqual(budget.calls, 32 - 9)  # Six bindings, handshake and two queries.
            self.assertFalse(out['recovery_required'])

    def test_source_loss_is_retained_but_not_resolved_and_old_generation_is_refused(self):
        with tempfile.TemporaryDirectory() as directory, Peer(('source_lost',), generation=12) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                out = owner.query()
            self.assertTrue(out['recovery_required'])
            self.assertTrue(out['clock_result']['requires_operator_attention'])
        with tempfile.TemporaryDirectory() as directory, Peer(generation=10) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                owner.query()
            self.assertEqual(peer.calls, ['Handshake'])
            self.assertFalse(f.outcome.exists())

    def test_dropped_reply_preserves_original_files_and_never_reconnects(self):
        with tempfile.TemporaryDirectory() as directory, Peer(('drop',)) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            before = f.snapshot()
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                owner.query()
            self.assertEqual(before, f.snapshot())
            self.assertEqual(peer.calls, ['Handshake', 'QueryRun'])

    def test_bad_receipt_nonce_and_identity_never_publish_outcome(self):
        for damage in ('receipt', 'nonce', 'key', 'software'):
            def change(name, values):
                if name != 'QueryRun': return
                if damage == 'receipt': values[10] = values[10][:-1] + bytes([values[10][-1] ^ 1])
                if damage == 'nonce': values[3] = b'wrong' * 6
                if damage == 'key': values[10] = record(dict(f.intent, idempotency_key='another-key'), 'stopped')
                if damage == 'software': values[7] = b'another-df'
            with tempfile.TemporaryDirectory() as directory, Peer(change=change) as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                    owner.query()
                self.assertFalse(f.outcome.exists())

    def test_authority_revoked_during_reply_cannot_publish_success(self):
        def change(name, values):
            if name == 'QueryRun': os.environ['DFMCP_RUN_TOKEN'] = 'z' * 32
        with tempfile.TemporaryDirectory() as directory, Peer(change=change) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                owner.query()
            self.assertFalse(f.outcome.exists())

    def test_output_reservation_and_call_allowance_cannot_be_skipped(self):
        with tempfile.TemporaryDirectory() as directory:
            f = Fixture(directory)
            with patch.dict(os.environ, env(), clear=True), patch.object(recovery, 'MAX_OUTPUT', 32768):
                with f.owner() as owner, patch.object(socket, 'socket', side_effect=AssertionError('network')):
                    with self.assertRaises(ValueError): owner.query()
        with tempfile.TemporaryDirectory() as directory, Peer() as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            budget = recovery.Budget(); budget.calls = 7
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner(budget=budget) as owner:
                owner.query()
            self.assertEqual(peer.calls, ['Handshake'])
            self.assertFalse(f.outcome.exists())

    def test_query_transport_cannot_observe_prepare_commit_or_cancel(self):
        for operation in ('ObserveRun', 'PrepareRun', 'CommitRun', 'CancelRun'):
            with tempfile.TemporaryDirectory() as directory, Peer() as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                with patch.dict(os.environ, env(peer.address), clear=True), f.owner() as owner:
                    authority = recovery.Authority.load()
                    with context.Client(authority, owner.budget, owner.check, intent=f.intent, query=True) as client:
                        with self.assertRaises(ValueError): client.call(operation, clock.intent_fields(f.intent, True))
                self.assertEqual(peer.calls, ['Handshake'])

    def test_scope_replacement_during_reply_fences_before_retention(self):
        def change(name, values):
            if name == 'QueryRun':
                raw = f.sidecar.read_bytes(); f.sidecar.rename(f.root / 'old'); private_write(f.sidecar, raw)
        with tempfile.TemporaryDirectory() as directory, Peer(change=change) as peer:
            f = Fixture(directory, peer.address); peer.intent = f.intent
            with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(ValueError), f.owner() as owner:
                owner.query()
            self.assertFalse(f.outcome.exists())

    def test_failed_sync_and_torn_publication_do_not_acknowledge_or_repair(self):
        for fault in ('sync', 'torn'):
            with tempfile.TemporaryDirectory() as directory, Peer() as peer:
                f = Fixture(directory, peer.address); peer.intent = f.intent
                original_write = os.write
                def torn(fd, raw):
                    original_write(fd, raw[:17]); raise OSError('injected incomplete output')
                effect = patch.object(storage.os, 'fsync', side_effect=OSError('sync failed')) if fault == 'sync' else patch.object(storage.os, 'write', side_effect=torn)
                with patch.dict(os.environ, env(peer.address), clear=True), self.assertRaises(OSError), f.owner() as owner, effect:
                    owner.query()
                self.assertTrue(f.outcome.exists())
                bytes_after = f.outcome.read_bytes()
                if fault == 'torn':
                    with self.assertRaises(ValueError), f.owner(): pass
                else:
                    with f.owner() as owner:
                        out = owner.inspect()
                    self.assertFalse(out['clock_result']['storage_acknowledged_this_call'])
                self.assertEqual(f.outcome.read_bytes(), bytes_after)
                self.assertEqual(peer.calls, ['Handshake', 'QueryRun'])

    def test_actual_cli_subprocess_queries_without_route_batch_files(self):
        with tempfile.TemporaryDirectory() as directory, Peer() as peer:
            f = Fixture(directory, peer.address, final=True); peer.intent = f.intent
            result = subprocess.run([sys.executable, '-B', recovery.__file__, *f.args('query')],
                                    env=env(peer.address), capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
            self.assertEqual(len(result.stdout.splitlines()), 1)
            packet = json.loads(result.stdout)
            self.assertTrue(packet['ok'])
            self.assertFalse(packet['result']['original_batch_custody_verified'])
            self.assertEqual(peer.calls, ['Handshake', 'QueryRun'])
            self.assertNotIn(TOKEN, result.stdout.decode())


if __name__ == '__main__':
    unittest.main()
