"""Execute Python evidence/RPC logic against independent bytes and joined TCP doubles.

No real DFHack, SDK, generated protobuf, Rust/MCP or production qualification.
Reference vectors reconstruct the existing native bridge suite's golden inputs.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import itertools
import os
import socket
import struct
import threading
import time
import unittest
from unittest.mock import patch

import excavation_run_wire as w
import excavation_run_rpc as rpc

REGION = w.Region(15, 15, 2, 2, 2)
SPEC = w.Spec(100, 1000, 2, 2, 1, 10)
SECRET = b's' * 32


def reference_hash(domain, value):
    return hashlib.sha256(domain + b'\0' + value).digest()


def sized(value):
    return struct.pack('>H', len(value)) + value


def capture(tick=806500, sequence=3, paused=1, cell=b'\x02\x02\x00\x01',
            generation=41, folder=b'region1', region=(15, 15, 2, 2, 2), dimensions=(64, 64, 8)):
    return (b'DFMEC018DFMRO013' + struct.pack('>QQQBBB', generation, sequence, tick, 1, 1, paused)
            + struct.pack('>4I', 2, *dimensions) + sized(folder)
            + struct.pack('>5IH', *region, region[3] * region[4]) + cell * region[3] * region[4])


def record(key='golden', phase=0, reason=0, trigger=0, known=None, tick=806504,
           stable=None, first=None, counted=None, last=None, sample=None, before=None, spec=None):
    before = capture() if before is None else before
    spec = SPEC.values() if spec is None else spec
    before_tick = struct.unpack('>Q', before[32:40])[0]
    spec_raw = struct.pack('>6I', *spec)
    plan = reference_hash(b'dfmcp-excavation-run-plan/1', spec_raw + before)
    token = reference_hash(b'dfmcp-excavation-run-token/1', sized(key.encode()) + plan)[:16]
    if known is None:
        known = int(phase in (1, 2, 3))
    if stable is None:
        stable = spec[2] if trigger == 1 else 0
    if first is None:
        first = before_tick + 1 if stable else 0
    if last is None:
        last = struct.unpack('>Q', sample[32:40])[0] if sample else before_tick
    if counted is None:
        counted = last if stable else before_tick
    body = (b'DFMER018' + sized(key.encode()) + spec_raw + sized(before) + plan + token
            + bytes((phase, reason, int(phase in (1, 2, 3, 5)), int(phase == 3), known))
            + struct.pack('>QBIQQQB', tick if known else 0, trigger, stable, first, counted, last, int(sample is not None)))
    if sample is not None:
        body += sized(sample)
    return body + reference_hash(b'dfmcp-excavation-run-receipt/1', body)


def plan(key='golden'):
    return w.Plan(key, SPEC, w.Capture.decode(capture()))


def stopped(key='golden', before=None):
    return record(key=key, phase=3, reason=3, trigger=1, before=before,
                  sample=capture(806504, 4, 0, b'\x02\x03\x00\x00'))


def message(fields):
    def number(n):
        out = []
        while n > 127:
            out.append(n % 128 + 128)
            n //= 128
        return bytes(out + [n])
    result = b''
    for key, value in fields.items():
        if isinstance(value, bytes):
            result += number(key * 8 + 2) + number(len(value)) + value
        else:
            result += number(key * 8) + number(value)
    return result


class NativeDouble:
    def __init__(self, connections=1, lose_on=None, reply_hook=None, query_raw=None, alias=False, notifications=0, delay=0):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(4)
        self.listener.settimeout(2)
        self.address = self.listener.getsockname()
        self.connections, self.lose_on = connections, lose_on
        self.reply_hook, self.query_raw = reply_hook, query_raw
        self.alias, self.notifications, self.delay = alias, notifications, delay
        self.calls, self.requests, self.error = [], [], None
        self.effects = 0
        self.thread = threading.Thread(target=self.serve, daemon=False)

    @staticmethod
    def read(sock, size):
        out = b''
        while len(out) < size:
            part = sock.recv(size - len(out))
            if not part:
                raise EOFError()
            out += part
        return out

    def serve(self):
        try:
            for _ in range(self.connections):
                with self.listener.accept()[0] as sock:
                    sock.settimeout(2)
                    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    try:
                        assert self.read(sock, 12) == b'DFHack?\n\x01\x00\x00\x00'
                        if self.delay:
                            time.sleep(self.delay)
                        sock.sendall(b'DFHack!\n\x01\x00\x00\x00')
                        bindings = {}
                        while True:
                            method, size = struct.unpack('<h2xi', self.read(sock, 8))
                            assert 0 <= size <= 2048
                            fields = rpc.decode(self.read(sock, size), maximum=19)
                            if method == 0:
                                name = fields[1].decode()
                                assert name in rpc.METHODS
                                assert fields[2] == b'dfmcp.excavation_run.v1_18.Request'
                                assert fields[3] == b'dfmcp.excavation_run.v1_18.Reply'
                                assert fields[4] == b'dfmcp_excavation_run_v1_18'
                                method_id = 2 if self.alias else len(bindings) + 2
                                bindings[method_id] = name
                                response = {1: method_id}
                                for _ in range(self.notifications):
                                    sock.sendall(struct.pack('<h2xi', -3, 0))
                            else:
                                name = bindings[method]
                                self.calls.append(name)
                                self.requests.append(fields)
                                assert fields[1] == SECRET and fields[3] == 1 and fields[4] == 18
                                common = {1, 2, 3, 4}
                                shapes = {'Handshake': set(), 'ObserveRun': set(range(11, 16)),
                                          'PrepareRun': {5, 6, 7, 8, 9} | set(range(11, 20)),
                                          'CommitRun': {5, 9, 10}, 'CancelRun': {5, 9, 10}, 'QueryRun': {5, 9}}
                                assert set(fields) == common | shapes[name]
                                response = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 18, 6: 41,
                                            7: b'fake-df', 8: b'fake-dfhack', 11: 0, 12: 0}
                                key = fields.get(5, b'golden').decode()
                                if name == 'ObserveRun':
                                    assert tuple(fields[n] for n in range(11, 16)) == REGION.values()
                                    response[9] = capture()
                                elif name == 'PrepareRun':
                                    assert fields[8] == capture() and fields[9] == plan(key).digest
                                    assert (fields[6], fields[7], *(fields[n] for n in range(16, 20))) == SPEC.values()
                                    response[10] = record(key=key)
                                    response[12] = 1
                                elif name in ('CommitRun', 'CancelRun', 'QueryRun'):
                                    assert fields[9] == plan(key).digest
                                    if name != 'QueryRun':
                                        assert fields[10] == plan(key).token
                                    response[12] = 1
                                    if name == 'CommitRun':
                                        self.effects += 1
                                        response[10] = record(key=key, phase=1)
                                        response[11] = 1
                                    elif name == 'CancelRun':
                                        response[10] = record(key=key, phase=3, reason=3)
                                    elif self.query_raw == b'':
                                        response[12] = 0
                                    else:
                                        response[10] = self.query_raw if self.query_raw is not None else stopped(key)
                                        response[11] = int(w.Record.decode(response[10]).phase in ('running', 'stopping'))
                                if self.reply_hook:
                                    response = self.reply_hook(name, fields, response)
                                if name == self.lose_on:
                                    sock.sendall(struct.pack('<h2xi', -1, 512) + b'\x08')
                                    break
                            raw = message(response)
                            framed = struct.pack('<h2xi', -1, len(raw)) + raw
                            for start in range(0, len(framed), 7):
                                sock.sendall(framed[start:start + 7])
                    except (EOFError, BrokenPipeError, ConnectionResetError):
                        pass
        except BaseException as error:
            self.error = error
        finally:
            self.listener.close()

    def environment(self, clock='1'):
        values = {rpc.OPT_IN: '1', rpc.TOKEN: SECRET.decode(), rpc.ENDPOINT: f'{self.address[0]}:{self.address[1]}'}
        if clock is not None:
            values[rpc.CLOCK] = clock
        return patch.dict(os.environ, values, clear=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.thread.join(5)
        if self.thread.is_alive():
            self.listener.close()
            self.thread.join(3)
            raise AssertionError('native double did not quiesce')
        if self.error:
            raise AssertionError('native double failed') from self.error


class WireTests(unittest.TestCase):
    def test_existing_native_golden_inputs(self):
        before = w.Capture.decode(capture())
        self.assertEqual(before.view()['matching_cells'], 0)
        value = w.Record.decode(stopped(), plan())
        self.assertTrue(value.pause_verified)
        self.assertEqual((value.trigger, value.stable_samples, value.first_stable_tick, value.counted_tick),
                         ('floor_observed', 2, 806501, 806504))
        self.assertEqual(value.sample.cells, ((2, 3, 0, 0),) * 4)
        self.assertEqual(value.plan.digest, reference_hash(b'dfmcp-excavation-run-plan/1', struct.pack('>6I', *SPEC.values()) + capture()))
        self.assertFalse(value.view()['continuous_stability_proved'])
        self.assertFalse(value.view()['retry_permitted'])

    def test_every_prefix_and_byte_corruption(self):
        raw = stopped()
        for index in range(len(raw)):
            with self.subTest(index=index), self.assertRaises(w.Rejected):
                w.Record.decode(raw[:index])
            with self.subTest(corruption=index), self.assertRaises(w.Rejected):
                w.Record.decode(raw[:index] + bytes([raw[index] ^ 128]) + raw[index + 1:])
        with self.assertRaises(w.Rejected):
            w.Record.decode(raw + b'\0')

    def test_hidden_cells_carry_no_attributes(self):
        for cell in (b'\0', b'\1'):
            result = w.Capture.decode(capture(cell=cell))
            self.assertEqual(result.cells, ((cell[0],),) * 4)
            with self.assertRaises(w.Rejected):
                w.Plan('golden', SPEC, result)
            with self.assertRaises(w.Rejected):
                w.Capture.decode(capture(cell=cell + b'\2\3\4'))

    def test_geometry_and_capture_bounds(self):
        for region in ((0, 0, 0, 0, 1), (0, 0, 0, 9, 1), (32767, 0, 0, 2, 1), (0, 0, 32768, 1, 1)):
            with self.subTest(region=region), self.assertRaises(w.Rejected):
                w.Region(*region)
        for raw in (capture(dimensions=(16, 16, 8)), capture(folder=b'\xff'), capture(folder=b'a\0b'),
                    capture(cell=b'\2\x09\0\0'), capture(cell=b'\2\3\x08\0'), b'x' * 1025):
            with self.assertRaises(w.Rejected):
                w.Capture.decode(raw)

    def test_plan_preconditions_and_public_field_substitution(self):
        for before in (capture(paused=0), capture(cell=b'\2\3\0\0'), capture(cell=b'\2\2\1\1'),
                       capture(sequence=2**64 - 1), capture(tick=w.MAX_TICK - 99)):
            with self.assertRaises(w.Rejected):
                w.Plan('golden', SPEC, w.Capture.decode(before))
        with self.assertRaises(w.Rejected):
            w.Plan('golden', SPEC, replace(w.Capture.decode(capture()), tick=1))
        for spec in ((2, 1, 2, 0, 1, 1), (100, 1, 2, 99, 1, 10), (100, 1, 0, 0, 1, 10),
                     (100, 1, 1, 0, 2, 1), (True, 1, 1, 0, 1, 1)):
            with self.assertRaises(w.Rejected):
                w.Spec(*spec)

    def test_all_phase_reason_combinations(self):
        allowed = ({0}, {0}, {1, 2, 3, 5, 6, 8}, {1, 2, 3, 4, 5, 6, 8}, {3, 7, 9}, {7})
        for phase, reason in itertools.product(range(6), range(10)):
            raw = record(phase=phase, reason=reason)
            if reason in allowed[phase]:
                self.assertEqual(w.Record.decode(raw).phase, w.PHASES[phase])
            else:
                with self.assertRaises(w.Rejected):
                    w.Record.decode(raw)

    def test_rehashed_false_goal_evidence(self):
        good = capture(806504, 4, 0, b'\2\3\0\0')
        for changes in ({'stable': 1}, {'first': 806504}, {'first': 806500}, {'counted': 806503},
                        {'sample': capture(806504, 4, 0)}, {'sample': capture(806504, 5, 0, b'\2\3\0\0')},
                        {'sample': capture(806504, 4, 1, b'\2\3\0\0')},
                        {'sample': capture(806504, 4, 0, b'\2\3\0\0', folder=b'other')}, {'reason': 1}):
            args = {'phase': 3, 'reason': 3, 'trigger': 1, 'sample': good, **changes}
            with self.subTest(changes=changes), self.assertRaises(w.Rejected):
                w.Record.decode(record(**args))

    def test_safety_triggers_and_source_loss_are_not_goal_completion(self):
        for trigger, cell in ((4, b'\1'), (5, b'\2\3\1\0')):
            value = w.Record.decode(record(phase=2, reason=3, trigger=trigger,
                                          sample=capture(806504, 4, 0, cell)))
            self.assertFalse(value.pause_verified)
            self.assertFalse(value.view()['sampled_floor_condition_reported'])
        loss = w.Record.decode(record(phase=5, reason=7, trigger=1,
                               sample=capture(806504, 4, 0, b'\2\3\0\0')))
        self.assertTrue(loss.terminal)
        self.assertFalse(loss.resolved)
        self.assertFalse(loss.pause_verified)
        self.assertTrue(loss.view()['operator_attention_required'])

    def test_largest_native_shape_and_exact_plan_binding(self):
        before = capture(folder=b'x' * 512, region=(0, 0, 0, 8, 8))
        after = capture(806504, 4, 0, b'\2\3\0\0', folder=b'x' * 512, region=(0, 0, 0, 8, 8))
        raw = record(key='x' * 128, phase=3, reason=3, trigger=1, sample=after, before=before)
        self.assertEqual(len(raw), 1991)
        self.assertEqual(len(w.Record.decode(raw).sample.cells), 64)
        with self.assertRaises(w.Rejected):
            w.Record.decode(stopped(), plan('different'))

    def test_terminal_immutability_and_phase_regressions(self):
        prepared, running, final = map(w.Record.decode, (record(), record(phase=1), stopped()))
        w.successor(prepared, running)
        w.successor(running, final)
        w.successor(final, final)
        for old, new in ((running, prepared), (final, running), (final, w.Record.decode(record(phase=3, reason=2)))):
            with self.assertRaises(w.Rejected):
                w.successor(old, new)


class RpcTests(unittest.TestCase):
    def client(self, server, **kwargs):
        return rpc.Client(rpc.Authority.load(), rpc.Budget(5000, **kwargs), REGION)

    def test_fragmented_lifecycle_uses_exact_native_methods(self):
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            self.assertEqual(client.observe().capture, plan().before)
            self.assertEqual(client.prepare(plan()).record.phase, 'prepared')
            self.assertEqual(client.commit(plan()).record.phase, 'running')
            self.assertEqual(client.query(plan()).record.trigger, 'floor_observed')
            self.assertEqual(server.effects, 1)
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.calls, ['Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun'])

    def test_query_or_import_cannot_authorize_commit(self):
        with NativeDouble(query_raw=record()) as server, server.environment(), self.client(server) as client:
            client.query(plan())
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            with self.assertRaises(w.Rejected):
                client.prepare(plan())
            self.assertEqual(server.effects, 0)

    def test_lost_commit_reconnect_only_queries(self):
        with NativeDouble(connections=2, lose_on='CommitRun') as server, server.environment():
            with self.client(server) as client:
                client.prepare(plan())
                with self.assertRaises(w.Rejected):
                    client.commit(plan())
                with self.assertRaises(w.Rejected):
                    client.commit(plan())
            with self.client(server) as client:
                self.assertEqual(client.query(plan()).record.phase, 'stopped')
            self.assertEqual(server.effects, 1)
            self.assertEqual(server.calls.count('CommitRun'), 1)

    def test_cancel_consumes_preparation_and_never_commits(self):
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            self.assertEqual(client.cancel(plan()).record.phase, 'stopped')
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.effects, 0)

    def test_revoking_clock_keeps_query_and_cancel_available(self):
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            os.environ[rpc.CLOCK] = '0'
            self.assertTrue(client.cancel(plan()).record.terminal)
            self.assertNotIn('CommitRun', server.calls)
        with NativeDouble() as server, server.environment(), self.client(server) as client:
            client.prepare(plan())
            os.environ[rpc.CLOCK] = '0'
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.effects, 0)

    def test_absence_does_not_grant_retry(self):
        with NativeDouble(query_raw=b'') as server, server.environment(clock=None), self.client(server) as client:
            self.assertIsNone(client.query(plan()).record)
            with self.assertRaises(w.Rejected):
                client.commit(plan())
            self.assertEqual(server.calls, ['Handshake', 'QueryRun'])

    def test_wrong_nonce_fields_types_identity_and_aliases_poison(self):
        changes = ({3: b'wrong'}, {5: 13}, {6: b'41'}, {11: 2}, {12: 257}, {13: 1}, {9: b'extra'})
        for change in changes:
            with self.subTest(change=change), NativeDouble(reply_hook=lambda n, f, r: {**r, **change}) as server, server.environment():
                with self.assertRaises(w.Rejected):
                    self.client(server)
        with NativeDouble(alias=True) as server, server.environment(), self.assertRaises(w.Rejected):
            self.client(server)
        with NativeDouble(reply_hook=lambda n, f, r: {**r, 6: 42} if n == 'ObserveRun' else r) as server, server.environment(), self.client(server) as client:
            with self.assertRaises(w.Rejected):
                client.observe()
            self.assertTrue(client._closed)

    def test_notifications_calls_and_whole_handshake_deadline(self):
        with NativeDouble(notifications=9) as server, server.environment(), self.assertRaises(w.Rejected):
            self.client(server)
        with NativeDouble() as server, server.environment(), self.client(server, max_calls=7) as client:
            with self.assertRaises(w.Rejected):
                client.observe()
            self.assertEqual(server.calls, ['Handshake'])
        with NativeDouble(delay=0.04) as server, server.environment():
            with self.assertRaises((w.Rejected, TimeoutError)):
                rpc.Client(rpc.Authority.load(), rpc.Budget(5), REGION)

    def test_noncanonical_protobuf_rejected(self):
        for value in (0, 127, 128, 2**32 - 1, 2**64 - 1):
            self.assertEqual(rpc.decode(message({1: value, 2: b'abc'})), {1: value, 2: b'abc'})
            self.assertEqual(rpc.encode({1: value, 2: b'abc'}), message({1: value, 2: b'abc'}))
        for raw in (b'\x08\x80\0', b'\x08\x80', b'\x08' + b'\xff' * 10, b'\x00', b'\x0b',
                    b'\x08\1\x08\1', b'\x12\4abc', b'\x68\1', b'x' * 4097):
            with self.assertRaises(w.Rejected):
                rpc.decode(raw)

    def test_authority_and_endpoint_bounds(self):
        for value in ('example.com:5000', '192.168.1.1:5000', '127.0.0.1:0', '127.0.0.1:05000', '127.0.0.1:65536', '[::1]:5000'):
            with self.assertRaises(ValueError):
                rpc.endpoint(value)
        good = {rpc.OPT_IN: '1', rpc.TOKEN: SECRET.decode(), rpc.CLOCK: '1'}
        with patch.dict(os.environ, good, clear=True):
            value = rpc.Authority.load(True)
            self.assertNotIn(SECRET.decode(), repr(value))
            os.environ['DFMCP_ADMITTED_BRIDGE_PROTOCOL'] = '1.0'
            with self.assertRaises(w.Rejected):
                value.guard('QueryRun')


if __name__ == '__main__':
    unittest.main(verbosity=2)
