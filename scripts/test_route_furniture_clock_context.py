"""Actual run/1.13 client and private files against a bounded joined TCP peer.

Native wire fields/records are independently encoded here. The original route
batch and route observation are explicit ports, not a DFHack or batch-store test.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field, replace
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import struct
import sys
import tempfile
import threading
import time
import types
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as storage
from furniture_plan import FurniturePlan, Step, canonical, progress, require
from route_furniture_review import Source
import route_furniture_clock_context as context


class Budget:
    """Same foreground fields and semantics as the production route budget."""
    def __init__(self, timeout_ms=60000):
        clock.integer(timeout_ms, 1, 60000)
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.calls, self.network_bytes = 560, 40 * 1024 * 1024
        self.disk_bytes, self.work_steps = 1024 * 1024 * 1024, 20_000_000
        self.spent = dict(calls=0, network_bytes=0, disk_bytes=0)

    def remaining(self):
        left = self.deadline - time.monotonic()
        require(left > 0, 'deadline')
        return left

    def work(self):
        self.remaining()
        require(self.work_steps > 0, 'work')
        self.work_steps -= 1

    def reserve(self, kind, count):
        self.remaining()
        clock.integer(count, 0, 1024 * 1024 * 1024)
        require(kind in self.spent and count <= getattr(self, kind), 'allowance')
        setattr(self, kind, getattr(self, kind) - count)
        self.spent[kind] += count


class Batch:
    """Explicit original-batch custody port; real plan and Source semantics."""
    def __init__(self, address, size=3):
        self.address, self.writable, self.closed = address, False, False
        self.budget = Budget()
        self.plan = FurniturePlan(tuple(Step(f's{i:02}', 'bed', 10 + i, (i + 2, 2, 0))
                                        for i in range(size)))
        self.id = 'b' * 64
        self.review = types.SimpleNamespace(digest='c' * 64)
        self.source = Source(f'{address[0]}:{address[1]}', 'df-fixture', 'dfhack-fixture',
                             7, 9, 'a' * 64, 'd' * 64, 10, (64, 64, 2),
                             (0, 0, 0), (32, 32, 1), (100, 100, 100))
        self.records = {'s00': 'placed'}
        self.stopped, self.corrupt = False, False
        self.reads = 0
        self.route_status = 'construction_pending'
        self.current_tick = 10
        self.on_review = lambda state: None

    def check(self):
        self.budget.work()
        require(not self.corrupt and not self.closed, 'original custody lost')

    def audit(self):
        self.check()
        state = progress(self.plan, self.records)
        state.update(batch_id=self.id, review_digest=self.review.digest,
            inventory_verified=True,
            inventory_digest=hashlib.sha256(canonical([self.id, self.records, self.stopped])).hexdigest(),
            stopped=self.stopped, advance_allowed=state['status'] == 'ready' and not self.stopped)
        return state

    def review_step(self, authority):
        authority.guard('ReadPlacement')
        state = self.audit()
        self.reads += 1
        ready = self.route_status == 'ready'
        report = {'batch_id': self.id, 'status': self.route_status,
            'source': replace(self.source, tick=self.current_tick).json(),
            'construction_prefix': {'status': 'quiescent' if ready else 'pending'}}
        report['evidence_digest'] = hashlib.sha256(canonical(report)).hexdigest()
        state.update(route_frontier=report, advance_allowed=ready,
                     expected_plan='e' * 64 if ready else None,
                     confirm_review='f' * 64 if ready else None)
        self.on_review(state)
        return state

    def close(self):
        self.closed = True


@dataclass(frozen=True)
class RouteAuthority:
    address: tuple[str, int]
    token: bytes = field(repr=False)
    operations_token: bytes = field(repr=False)
    map_token: bytes = field(repr=False)
    allow_place: bool = False


@contextmanager
def workflow():
    """Load complete production progress source with ONLY route/custody ports."""
    route = types.ModuleType('route_furniture_batch')
    route.Authority, route.Budget = RouteAuthority, Budget
    route.OPT_IN = 'DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH'
    route.ENDPOINT = 'DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT'
    route.PLACE = 'DFMCP_ROUTE_FURNITURE_ALLOW_PLACE'
    route.MAP_TOKEN, route.OPS_TOKEN = 'DFMCP_MAP_TOKEN', 'DFMCP_OPERATIONS_PAGED_TOKEN'
    route.native = types.SimpleNamespace(TOKEN='DFMCP_BUILD_TOKEN', endpoint=clock.endpoint)
    route.ENVIRONMENT = (route.OPT_IN, route.ENDPOINT, route.PLACE,
                         route.MAP_TOKEN, route.OPS_TOKEN, route.native.TOKEN)
    route.review_step = lambda batch, authority: batch.review_step(authority)
    route.packet = lambda operation, state, error=False: {
        'schema': 'fixture-outer-route-packet', 'operation': operation, 'ok': not error,
        'result': state, 'agent_turn': {'briefing': {}},
        **({'error': {'code': 'ROUTE_BATCH_REFUSED'}} if error else {})}
    parent = types.ModuleType('route_furniture_batch_store')
    parent.Batch = Batch
    name = '_dfmcp_progress_under_test'
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name('route_furniture_progress.py'))
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module, 'route_furniture_batch': route,
                                  'route_furniture_batch_store': parent}):
        spec.loader.exec_module(module)
        yield module, route


def environment(address):
    endpoint = f'{address[0]}:{address[1]}'
    return {'DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH': '1',
        'DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT': endpoint,
        'DFMCP_BUILD_TOKEN': 'b' * 32, 'DFMCP_MAP_TOKEN': 'm' * 32,
        'DFMCP_OPERATIONS_PAGED_TOKEN': 'o' * 32,
        'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1', 'DFMCP_RUN_ENDPOINT': endpoint,
        'DFMCP_RUN_TOKEN': 'r' * 32}


def uv(n):
    out = bytearray()
    while n > 127:
        out.append(n % 128 + 128)
        n //= 128
    return bytes(out + bytes([n]))


def pb(fields):
    out = bytearray()
    for number, value in sorted(fields.items()):
        if type(value) is bytes:
            out += uv(number * 8 + 2) + uv(len(value)) + value
        else:
            out += uv(number * 8) + uv(value)
    return bytes(out)


def parse_pb(raw):
    offset, fields = 0, {}
    def take():
        nonlocal offset
        value, bits = 0, 0
        while True:
            byte = raw[offset]
            offset += 1
            value |= (byte & 127) << bits
            if byte < 128:
                return value
            bits += 7
    while offset < len(raw):
        tag = take()
        if tag % 8 == 0:
            value = take()
        else:
            assert tag % 8 == 2
            width = take()
            value, offset = raw[offset:offset + width], offset + width
        assert tag // 8 not in fields
        fields[tag // 8] = value
    return fields


def native_observation(generation=21, sequence=3, tick=10):
    return b'DFMRO013' + struct.pack('>QQQBBB', generation, sequence, tick, 1, 1, 1)


def native_record(key, ticks, wall, observation, phase, reason=0, observed=None):
    kb = key.encode('ascii')
    framed_key = struct.pack('>H', len(kb)) + kb
    plan = hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + struct.pack('>II', ticks, wall) + observation).digest()
    token = hashlib.sha256(b'dfmcp-bounded-run-token/1\0' + framed_key + plan).digest()[:16]
    raw = (b'DFMRE013' + framed_key + struct.pack('>II', ticks, wall) + observation + plan + token
           + struct.pack('>BBBBBQ', phase, reason, phase in (1, 2, 3, 5), phase == 3,
                         observed is not None, 0 if observed is None else observed))
    return raw + hashlib.sha256(b'dfmcp-bounded-run-receipt/1\0' + raw).digest()


class Peer:
    """Finite loopback fixture, bounded accept/read waits and always joined."""
    METHODS = ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun', 'CancelRun')
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(8)
        self.listener.settimeout(.1)
        self.address = self.listener.getsockname()
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.serve)
        self.events, self.errors, self.records = [], [], {}
        self.generation, self.sequence, self.tick = 21, 3, 10
        self.versions = (b'df-fixture', b'dfhack-fixture')
        self.owner_active, self.retained_records = False, 0
        self.connections, self.sent, self.received = 0, 0, 0
        self.on_call = lambda peer, operation, fields, connection: None
        self.commit_phase = 'stopped'
        self.drop_commit = False
        self.notifications = 0
        self.after_commit = lambda: None

    def __enter__(self):
        self.thread.start()
        return self

    def read(self, connection, n):
        out = bytearray()
        while len(out) < n:
            part = connection.recv(n - len(out))
            if not part:
                raise EOFError()
            self.received += len(part)
            out += part
        return bytes(out)

    def send(self, connection, raw):
        # Fragment every header/payload without relying on sleeps.
        for begin in range(0, len(raw), 37):
            part = raw[begin:begin + 37]
            connection.sendall(part)
            self.sent += len(part)

    def reply(self, connection, fields):
        raw = pb(fields)
        self.send(connection, struct.pack('<h2xi', -1, len(raw)) + raw)

    def serve(self):
        while not self.stop.is_set():
            try:
                connection, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self.connections += 1
            with connection:
                connection.settimeout(2)
                try:
                    self.session(connection)
                except (EOFError, ConnectionResetError, BrokenPipeError):
                    pass  # Client refusal/cancellation intentionally closes early.
                except BaseException as exc:
                    self.errors.append(exc)

    def session(self, connection):
        assert self.read(connection, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(connection, b'DFHack!\n' + struct.pack('<i', 1))
        for ordinal, name in enumerate(self.METHODS, 2):
            method, width = struct.unpack('<h2xi', self.read(connection, 8))
            assert method == 0 and 0 <= width <= 2048
            fields = parse_pb(self.read(connection, width))
            assert fields == {1: name.encode(), 2: b'dfmcp.run.v1_13.Request',
                              3: b'dfmcp.run.v1_13.Reply', 4: b'dfmcp_run_v1_13'}
            self.reply(connection, {1: ordinal})
        while True:
            method, width = struct.unpack('<h2xi', self.read(connection, 8))
            assert 2 <= method < 8 and 0 <= width <= 2048
            fields = parse_pb(self.read(connection, width))
            assert fields[1] == b'r' * 32 and len(fields[2]) == 32
            assert (fields[3], fields[4]) == (1, 13)
            operation = self.METHODS[method - 2]
            self.events.append((operation, dict(fields)))
            self.on_call(self, operation, fields, self.connections)
            result = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 13, 6: self.generation,
                      7: self.versions[0], 8: self.versions[1],
                      11: int(self.owner_active), 12: self.retained_records}
            if operation == 'ObserveRun':
                result[9] = native_observation(self.generation, self.sequence, self.tick)
            elif operation == 'PrepareRun':
                key = fields[5].decode()
                assert key not in self.records
                self.records[key] = (fields[6], fields[7], fields[8], 'prepared')
                result[10] = native_record(key, fields[6], fields[7], fields[8], 0)
            elif operation == 'CommitRun':
                key = fields[5].decode()
                ticks, wall, before, status = self.records[key]
                assert status == 'prepared'
                self.records[key] = (ticks, wall, before, self.commit_phase)
                result[11] = int(self.commit_phase == 'running')
                phase = 1 if self.commit_phase == 'running' else 3
                end_tick = struct.unpack_from('>Q', before, 24)[0] + ticks
                result[10] = native_record(key, ticks, wall, before, phase,
                                          0 if phase == 1 else 1, end_tick)
                if phase == 3:
                    self.tick, self.sequence = end_tick, self.sequence + 1
                    self.after_commit()
                if self.drop_commit:
                    return
            elif operation == 'QueryRun':
                key = fields[5].decode()
                if key in self.records:
                    ticks, wall, before, status = self.records[key]
                    phase = {'prepared': 0, 'running': 1, 'stopped': 3, 'source_lost': 5}[status]
                    result[11] = int(status == 'running')
                    result[10] = native_record(key, ticks, wall, before, phase,
                        1 if phase == 3 else 7 if phase == 5 else 0,
                        struct.unpack_from('>Q', before, 24)[0] + ticks if phase in (1, 3) else None)
                    if phase == 3:
                        self.tick = struct.unpack_from('>Q', before, 24)[0] + ticks
                        self.after_commit()
            elif operation != 'Handshake':
                raise AssertionError('unexpected mutation: ' + operation)
            for _ in range(self.notifications):
                self.send(connection, struct.pack('<h2xi', -3, 1024) + b'X' * 1024)
            self.reply(connection, result)

    def __exit__(self, kind, *_):
        self.stop.set()
        self.listener.close()
        self.thread.join(3)
        assert not self.thread.is_alive(), 'test peer failed to quiesce'
        if kind is None and self.errors:
            raise AssertionError(self.errors)

    @property
    def operations(self):
        return [name for name, _ in self.events]


@contextmanager
def fixture():
    with workflow() as (module, route), Peer() as peer, tempfile.TemporaryDirectory() as folder:
        os.chmod(folder, 0o700)
        with patch.dict(os.environ, environment(peer.address), clear=True):
            batch = Batch(peer.address)
            peer.after_commit = lambda: setattr(batch, 'current_tick', peer.tick)
            authority = module.ProgressAuthority.load()
            yield module, route, peer, batch, authority, Path(folder)


def reviewed(module, batch, authority, ticks=20, wall=50):
    state = module.review_progress(batch, authority, ticks, wall)
    assert state['progress_review']['confirm_progress'] is not None
    batch.budget = Budget()  # Separate explicitly requested top-level invocation.
    return state['progress_review']['confirm_progress']


class ClockContextTests(unittest.TestCase):
    def test_review_binds_exact_clock_under_real_native_transport(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            state = m.review_progress(batch, auth, 20, 50)
            scope = context.Scope.create(state['progress_review']['scope'])
            self.assertEqual(scope.digest, state['progress_review']['confirm_progress'])
            self.assertEqual(scope.value()['clock_observation_hex'], native_observation().hex())
            self.assertEqual(peer.operations, ['Handshake', 'ObserveRun'])
            self.assertEqual(batch.reads, 2)
            self.assertFalse(state['advance_allowed'])
            self.assertEqual(batch.budget.spent['calls'], 8)
            self.assertGreater(batch.budget.spent['network_bytes'], 100)
            self.assertEqual(list(folder.iterdir()), [])

    def test_complete_single_clock_effect_has_real_synced_scope_intent_and_outcome(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            path = m._record_path(str(folder), token)
            boundary_files = []
            def check(peer, operation, fields, connection):
                if operation in ('PrepareRun', 'CommitRun'):
                    scope = context.Scope(Path(str(path) + context.SCOPE_SUFFIX).read_bytes()[:-1])
                    intent = json.loads(path.read_bytes())['intent']
                    self.assertEqual(scope.intent(), intent)
                    self.assertEqual(scope.digest, token)
                    boundary_files.append(operation)
            peer.on_call = check
            result = m.progress(batch, auth, 20, 50, token, str(folder))
            record = result['construction_progress']
            self.assertEqual(boundary_files, ['PrepareRun', 'CommitRun'])
            self.assertEqual(peer.operations.count('CommitRun'), 1)
            self.assertTrue(record['clock_terminal_pause_verified'])
            self.assertTrue(record['clock_result']['storage_acknowledged_this_call'])
            self.assertEqual(record['post_progress_review']['status'], 'construction_pending')
            self.assertFalse(record['construction_completion_proven'])
            self.assertFalse(result['advance_allowed'])
            self.assertEqual(set(p.name for p in folder.iterdir()),
                             {path.name, path.name + storage.SUFFIX, path.name + context.SCOPE_SUFFIX})
            self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in folder.iterdir()))
            self.assertEqual(batch.budget.spent['calls'], 18)  # Review eight; start ten.

    def test_different_clock_snapshot_or_software_cannot_reuse_consent(self):
        for field in ('sequence', 'generation', 'tick', 'software', 'owner', 'capacity'):
            with self.subTest(field=field), fixture() as (m, route, peer, batch, auth, folder):
                token = reviewed(m, batch, auth)
                if field == 'software': peer.versions = (b'other', b'dfhack-fixture')
                elif field == 'owner': peer.owner_active = True
                elif field == 'capacity': peer.retained_records = 256
                else: setattr(peer, field, getattr(peer, field) + 1)
                with self.assertRaises(ValueError):
                    m.progress(batch, auth, 20, 50, token, str(folder))
                self.assertNotIn('PrepareRun', peer.operations)
                self.assertEqual(list(folder.iterdir()), [])

    def test_exact_approved_clock_rechecked_after_scope_sync_before_preparation(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            def change(peer, operation, fields, connection):
                if operation == 'ObserveRun' and connection == 3:
                    peer.sequence += 1
            peer.on_call = change
            with self.assertRaises(ValueError):
                m.progress(batch, auth, 20, 50, token, str(folder))
            self.assertNotIn('PrepareRun', peer.operations)
            self.assertEqual(len(list(folder.iterdir())), 1)  # Orphan scope deliberately remains.
            self.assertTrue(next(folder.iterdir()).name.endswith(context.SCOPE_SUFFIX))

    def test_authority_revocation_after_prepare_prevents_commit_and_preserves_intent(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.on_call = lambda p, op, f, n: os.environ.pop('DFMCP_RUN_TOKEN') if op == 'PrepareRun' else None
            with self.assertRaises(ValueError):
                m.progress(batch, auth, 20, 50, token, str(folder))
            self.assertEqual(peer.operations.count('PrepareRun'), 1)
            self.assertNotIn('CommitRun', peer.operations)
            self.assertEqual(len(list(folder.iterdir())), 2)

    def test_every_effect_rechecks_original_scope_and_intent_bytes(self):
        for target in ('scope', 'intent', 'batch', 'intent_copy'):
            with self.subTest(target=target), fixture() as (m, route, peer, batch, auth, folder):
                token = reviewed(m, batch, auth)
                path = m._record_path(str(folder), token)
                def change(peer, op, f, n):
                    if op == 'PrepareRun':
                        if target == 'batch': batch.corrupt = True
                        else:
                            changed = Path(str(path) + context.SCOPE_SUFFIX) if target == 'scope' else path
                            if target == 'intent_copy':
                                duplicate = folder / 'copy'
                                duplicate.write_bytes(changed.read_bytes())
                                duplicate.chmod(0o600)
                                duplicate.replace(changed)
                            else:
                                changed.write_bytes(changed.read_bytes() + b' ')
                peer.on_call = change
                with self.assertRaises(ValueError):
                    m.progress(batch, auth, 20, 50, token, str(folder))
                self.assertEqual(peer.operations.count('PrepareRun'), 1)
                self.assertNotIn('CommitRun', peer.operations)

    def test_lost_commit_reply_recovers_only_original_key_over_real_tcp(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            peer.drop_commit = True
            with self.assertRaises(ValueError):
                m.progress(batch, auth, 20, 50, token, str(folder))
            self.assertEqual(peer.operations.count('CommitRun'), 1)
            self.assertEqual(len(list(folder.iterdir())), 2)
            prior = len(peer.operations)
            batch.budget = Budget()
            value = m.query_progress(batch, auth, token, str(folder))
            self.assertEqual(peer.operations[prior:], ['Handshake', 'QueryRun'])
            self.assertTrue(value['construction_progress']['clock_terminal_pause_verified'])
            self.assertFalse(value['construction_progress']['clock_commit_invoked_this_call'])
            self.assertFalse(value['construction_progress']['construction_completion_proven'])
            self.assertEqual(value['construction_progress']['post_progress_review_status'], 'not_requested')
            self.assertEqual(len(list(folder.iterdir())), 3)

    def test_terminal_query_is_offline_without_authority_writes_or_freshness_claims(self):
        with fixture() as (m, route, peer, batch, auth, folder):
            token = reviewed(m, batch, auth)
            m.progress(batch, auth, 20, 50, token, str(folder))
            before = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in folder.iterdir()}
            reads, calls = batch.reads, len(peer.operations)
            batch.budget = Budget()
            with patch.dict(os.environ, {}, clear=True), patch.object(os, 'fsync', side_effect=AssertionError('offline write')):
                value = m.query_progress(batch, None, token, str(folder))
            self.assertEqual(len(peer.operations), calls)
            self.assertEqual(batch.reads, reads)
            self.assertEqual(before, {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in folder.iterdir()})
            self.assertFalse(value['construction_progress']['next_step_ready'])
            self.assertTrue(value['construction_progress']['current_pause_unproved'])
            self.assertFalse(value['construction_progress']['native_query_contacted'])

    def test_foreign_batch_or_missing_scope_cannot_adopt_clock_recovery(self):
        for target in ('batch', 'review', 'scope', 'source'):
            with self.subTest(target=target), fixture() as (m, route, peer, batch, auth, folder):
                token = reviewed(m, batch, auth)
                peer.drop_commit = True
                with self.assertRaises(ValueError): m.progress(batch, auth, 20, 50, token, str(folder))
                if target == 'batch': batch.id = 'a' * 64
                elif target == 'review': batch.review.digest = 'a' * 64
                elif target == 'source': batch.source = replace(batch.source, inventory_generation=10)
                else: Path(str(m._record_path(str(folder), token)) + context.SCOPE_SUFFIX).unlink()
                before = len(peer.operations)
                batch.budget = Budget()
                with self.assertRaises((OSError, ValueError)):
                    m.query_progress(batch, auth, token, str(folder))
                self.assertEqual(len(peer.operations), before)

    def test_original_new_run_callers_and_borrowed_directory_lock_remain_supported(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'original.run'
            with storage.new_run(path):  # Original API caller ignores the yielded fd.
                self.assertFalse(path.exists())
            with storage.new_run(path) as parent:
                self.assertIs(type(parent), int)
                import fcntl
                foreign = os.open(folder, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(foreign, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    os.close(foreign)


if __name__ == '__main__':
    unittest.main()
