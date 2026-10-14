"""Actual discovery/progress CLI and native clock TCP against joined test peers.

Original route acquisition, batch custody/budget and the outer packet are test
ports. Scope, inventory, clock client, native codecs, private files and locks are
real, unchanged production dependencies. This is not a DFHack qualification.
"""
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import struct
import sys
import threading
import types
import unittest
from unittest.mock import patch

import bounded_run_client as clock
import bounded_run_outcomes as outcomes
from furniture_plan import canonical
from route_furniture_review import Review, object_digest
from route_furniture_clock_context import Scope
import route_furniture_clock_inventory as history
from test_route_furniture_clock_inventory import (
    Batch as OriginalBatch, Budget as OriginalBudget, setup, save, scope_for, bytes_in,
)


class Budget(OriginalBudget):
    def __init__(self, timeout_ms=60000):
        super().__init__()
        self.calls = 560
        self.network_bytes = 40 * 1024 * 1024

    def reserve(self, kind, count):
        if kind == 'disk_bytes':
            return super().reserve(kind, count)
        self.remaining()
        if kind not in ('calls', 'network_bytes') or type(count) is not int or not 0 <= count <= getattr(self, kind):
            raise ValueError('whole operation allowance exceeded')
        setattr(self, kind, getattr(self, kind) - count)


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    token: bytes = field(repr=False)
    operations_token: bytes = field(repr=False)
    map_token: bytes = field(repr=False)
    allow_place: bool = False


def load_progress(source_path=None):
    route = types.ModuleType('route_furniture_batch')
    route.Authority, route.Budget = Authority, Budget
    route.OPT_IN = 'DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH'
    route.ENDPOINT = 'DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT'
    route.PLACE = 'DFMCP_ROUTE_FURNITURE_ALLOW_PLACE'
    route.MAP_TOKEN, route.OPS_TOKEN = 'DFMCP_MAP_TOKEN', 'DFMCP_OPERATIONS_PAGED_TOKEN'
    route.native = types.SimpleNamespace(TOKEN='DFMCP_BUILD_TOKEN', endpoint=clock.endpoint)
    route.ENVIRONMENT = (route.OPT_IN, route.ENDPOINT, route.PLACE, route.MAP_TOKEN, route.OPS_TOKEN, route.native.TOKEN)
    def packet(operation, state, error=False):
        return {'ok': not error, 'result': state, 'agent_turn': {
            'phase': 'inspect', 'briefing': {}, 'coverage': {}, 'active_work': [], 'uncertainty': []}}
    route.packet = packet
    runner = types.ModuleType('route_furniture_run')
    batch_store = types.ModuleType('route_furniture_batch_store')
    batch_store.Batch = OriginalBatch
    name = '_scoped_clock_inventory_progress_workflow'
    spec = importlib.util.spec_from_file_location(name, source_path or Path(__file__).with_name('route_furniture_progress.py'))
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module, 'route_furniture_batch': route,
                                 'route_furniture_run': runner, 'route_furniture_batch_store': batch_store}):
        spec.loader.exec_module(module)
    return module


def encode_varint(value):
    raw = []
    while value > 127:
        raw.append((value & 127) | 128)
        value >>= 7
    return bytes(raw + [value])


def encode_fields(values):
    raw = b''
    for number, value in sorted(values.items()):
        if type(value) is bytes:
            raw += encode_varint(number * 8 + 2) + encode_varint(len(value)) + value
        else:
            raw += encode_varint(number * 8) + encode_varint(value)
    return raw


def decode_fields(raw):
    offset = 0
    def number():
        nonlocal offset
        value = shift = 0
        while True:
            byte = raw[offset]; offset += 1
            value |= (byte & 127) << shift
            if byte < 128: return value
            shift += 7
            if shift > 63: raise AssertionError('test peer varint overflow')
    fields = {}
    while offset < len(raw):
        tag = number()
        field_number, kind = tag >> 3, tag & 7
        if field_number in fields: raise AssertionError('duplicate request field')
        if kind == 0: value = number()
        elif kind == 2:
            width = number(); value = raw[offset:offset + width]; offset += width
        else: raise AssertionError('unknown request wire type')
        fields[field_number] = value
    return fields


def read_exact(connection, size):
    result = b''
    while len(result) < size:
        part = connection.recv(size - len(result))
        if not part: raise EOFError
        result += part
    return result


def native_record(intent, phase):
    key = intent['idempotency_key'].encode('ascii')
    known = phase in ('running', 'stopped')
    reason = {'prepared': 0, 'running': 0, 'stopped': 1, 'refused': 9, 'source_lost': 7}[phase]
    raw = (b'DFMRE013' + struct.pack('>H', len(key)) + key
           + struct.pack('>II', intent['game_ticks'], intent['wall_ms'])
           + bytes.fromhex(intent['observation_hex']) + bytes.fromhex(intent['plan_digest_hex'])
           + bytes.fromhex(intent['prepare_token_hex'])
           + struct.pack('>BBBBBQ', ('prepared', 'running', 'stopping', 'stopped', 'refused', 'source_lost').index(phase),
                         reason, phase not in ('prepared', 'refused'), phase == 'stopped', known, 30 if known else 0))
    return raw + hashlib.sha256(b'dfmcp-bounded-run-receipt/1\0' + raw).digest()


class Peer:
    """Fixed native run profile only, joined and bounded; no background production worker."""
    methods = ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun', 'CancelRun')

    def __init__(self, sequence=1):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0)); self.listener.listen(4)
        self.listener.settimeout(0.05)
        self.address = self.listener.getsockname()
        self.before = b'DFMRO013' + struct.pack('>QQQBBB', 11, sequence, 10, 1, 1, 1)
        self.records, self.calls, self.errors = {}, [], []
        self.done = threading.Event()
        self.after_prepare = lambda: None
        self.drop_commit_reply = False
        self.thread = threading.Thread(target=self.serve)

    def __enter__(self):
        self.thread.start()
        return self

    def serve(self):
        try:
            while not self.done.is_set():
                try: connection, _ = self.listener.accept()
                except socket.timeout: continue
                except OSError:
                    if self.done.is_set(): return
                    raise
                with connection:
                    connection.settimeout(3)
                    try: self.handle(connection)
                    except (EOFError, ConnectionResetError, BrokenPipeError): pass
        except BaseException as error:
            self.errors.append(error)

    def handle(self, connection):
        assert read_exact(connection, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        connection.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        bound = []
        while not self.done.is_set():
            method, length = struct.unpack('<h2xi', read_exact(connection, 8))
            assert 0 <= length <= 2048
            fields = decode_fields(read_exact(connection, length))
            if method == 0:
                name = fields[1].decode('ascii')
                assert name == self.methods[len(bound)]
                assert fields[2] == b'dfmcp.run.v1_13.Request'
                assert fields[3] == b'dfmcp.run.v1_13.Reply' and fields[4] == b'dfmcp_run_v1_13'
                bound.append(name)
                payload = encode_fields({1: self.methods.index(name) + 2})
            else:
                assert tuple(bound) == self.methods
                name = self.methods[method - 2]
                assert name != 'CancelRun'
                assert fields[1] == b'r' * 32 and len(fields[2]) == 32
                assert fields[3] == 1 and fields[4] == 13
                self.calls.append(name)
                reply = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 13,
                         6: 11, 7: b'df', 8: b'dfhack', 11: 0, 12: len(self.records)}
                if name == 'ObserveRun': reply[9] = self.before
                elif name == 'PrepareRun':
                    key = fields[5].decode('ascii')
                    assert fields[8] == self.before
                    plan = hashlib.sha256(b'dfmcp-bounded-run-plan/1\0' + struct.pack('>II', fields[6], fields[7]) + fields[8]).digest()
                    assert fields[9] == plan
                    token = hashlib.sha256(b'dfmcp-bounded-run-token/1\0' + struct.pack('>H', len(fields[5])) + fields[5] + plan).digest()[:16]
                    intent = {'idempotency_key': key, 'game_ticks': fields[6], 'wall_ms': fields[7],
                              'observation_hex': fields[8].hex(), 'plan_digest_hex': plan.hex(), 'prepare_token_hex': token.hex()}
                    self.records[key] = (intent, 'prepared')
                    reply[10] = native_record(intent, 'prepared')
                    self.after_prepare()
                elif name in ('CommitRun', 'QueryRun'):
                    key = fields[5].decode('ascii')
                    if key in self.records:
                        intent, phase = self.records[key]
                        assert fields[9].hex() == intent['plan_digest_hex']
                        if name == 'CommitRun':
                            assert phase == 'prepared' and fields[10].hex() == intent['prepare_token_hex']
                            phase = 'stopped'; self.records[key] = (intent, phase)
                            if self.drop_commit_reply: return
                        reply[10] = native_record(intent, phase)
                payload = encode_fields(reply)
            connection.sendall(struct.pack('<h2xi', -1, len(payload)) + payload)

    def __exit__(self, *_):
        self.done.set(); self.listener.close(); self.thread.join(4)
        if self.thread.is_alive(): raise AssertionError('test peer did not join')
        if self.errors: raise self.errors[0]


def relocate(batch, address):
    endpoint = f'{address[0]}:{address[1]}'
    batch.address = address
    batch.source = replace(batch.source, endpoint=endpoint)
    value = batch.review.json(); value['source']['endpoint'] = endpoint
    raw = canonical(value)
    batch.review = Review.decode(raw, object_digest(raw))
    batch.budget = Budget()
    batch.closed = False
    batch.close = lambda: setattr(batch, 'closed', True)


@contextmanager
def authority_for(module, batch):
    env = {module.route.OPT_IN: '1', 'DFMCP_ALLOW_UNADMITTED_RUN_V1_13': '1',
           module.route.ENDPOINT: f'{batch.address[0]}:{batch.address[1]}',
           'DFMCP_RUN_ENDPOINT': f'{batch.address[0]}:{batch.address[1]}',
           'DFMCP_BUILD_TOKEN': 'b' * 32, module.route.OPS_TOKEN: 'o' * 32,
           module.route.MAP_TOKEN: 'm' * 32, 'DFMCP_RUN_TOKEN': 'r' * 32}
    with patch.dict(os.environ, env, clear=True):
        yield module.ProgressAuthority.load()


def frontier(batch, authority):
    authority.clock_guard()
    state = batch.audit()
    report = {'batch_id': batch.id, 'status': 'construction_pending', 'evidence_digest': 'd' * 64,
              'source': batch.source.json(), 'construction_prefix': {'status': 'pending'}}
    state.update(route_frontier=report, expected_plan=None, confirm_review=None, advance_allowed=False)
    return state, report


class WorkflowTests(unittest.TestCase):
    def setUp(self): self.module = load_progress()

    def cli(self, directory, batch, *extra, publish=None):
        outputs = []
        def open_batch(path, expected, budget, *, writable):
            self.assertEqual(expected, batch.id); self.assertFalse(writable)
            batch.budget = budget; return batch
        def output(raw):
            outputs.append(json.loads(raw))
            if publish is not None: return publish(raw)
            return 0
        argv = ['inventory', '--directory', '/original/test-port', '--batch-id', batch.id,
                '--clock-directory', str(directory), *extra]
        with patch.object(self.module, 'Batch', side_effect=open_batch) as opened, \
             patch.object(self.module, 'publish', side_effect=output):
            result = self.module.main(argv)
        return result, outputs, opened.call_count

    def test_inventory_cli_needs_no_confirmation_or_credentials_and_holds_both_owners(self):
        with setup() as (directory, batch):
            batch.close = lambda: setattr(batch, 'closed', True)
            scoped, _ = save(directory, batch, phase=None)
            def publish(_):
                with self.assertRaises(BlockingIOError):
                    with history.Inventory(directory, batch): pass
                self.assertFalse(getattr(batch, 'closed', False))
                return 0
            with patch.dict(os.environ, {}, clear=True), \
                 patch.object(socket, 'socket', side_effect=AssertionError('no native contact')):
                code, output, opened = self.cli(directory, batch, publish=publish)
            self.assertEqual((code, len(output), opened), (0, 1, 1))
            self.assertFalse(output[0]['result']['advance_allowed'])
            row = output[0]['result']['clock_inventory']['rows'][0]
            self.assertEqual(row['clock_record_id'], scoped.digest)
            self.assertTrue(row['query_supported'])
            self.assertEqual(output[0]['agent_turn']['active_work'][0]['clock_record_id'], scoped.digest)
            self.assertTrue(batch.closed)
            with history.Inventory(directory, batch): pass

    def test_inventory_cli_pages_and_returns_complete_counts_with_no_phantom_readiness(self):
        with setup() as (directory, batch):
            batch.close = lambda: None
            for i in range(3): save(directory, batch, sequence=i, phase=None)
            code, outputs, _ = self.cli(directory, batch, '--limit', '2', '--selection', 'pending')
            self.assertEqual(code, 0)
            page = outputs[0]['result']['clock_inventory']
            self.assertEqual(page['counts']['records'], 3)
            code, outputs, _ = self.cli(directory, batch, '--limit', '2', '--selection', 'pending',
                                         '--continuation', page['continuation'])
            self.assertEqual(code, 0)
            self.assertEqual(len(outputs[0]['result']['clock_inventory']['rows']), 1)
            self.assertFalse(outputs[0]['agent_turn']['briefing']['clock_start_authorized'])

    def test_invalid_inventory_options_refuse_before_opening_custody(self):
        with setup() as (directory, batch):
            batch.close = lambda: None
            for options in (('--limit', '0'), ('--limit', '65'), ('--ticks', '20'),
                            ('--confirm-progress', 'a' * 64), ('--wait-ms', '0'), ('--refresh-frontier',),
                            ('--continuation', 'x' * 2049)):
                with self.subTest(options=options):
                    code, output, opened = self.cli(directory, batch, *options)
                    self.assertEqual((code, len(output), opened), (2, 1, 0))
                    self.assertIsNone(output[0]['result'])

    def test_inventory_only_options_never_enable_effects_in_other_modes(self):
        with patch.object(self.module, 'Batch') as opened, patch.object(self.module, 'publish', return_value=0):
            for op in ('review', 'progress', 'query'):
                args = [op, '--directory', '/original', '--batch-id', 'a' * 64, '--selection', 'pending']
                self.assertEqual(self.module.main(args), 2)
            opened.assert_not_called()

    def test_short_or_failed_stdout_has_one_attempt_and_releases_lock(self):
        for raise_error in (False, True):
            with self.subTest(raise_error=raise_error), setup() as (directory, batch):
                batch.close = lambda: None
                def publish(_):
                    if raise_error: raise ValueError('short output')
                    return 2
                code, out, _ = self.cli(directory, batch, publish=publish)
                self.assertEqual((code, len(out)), (2, 1))
                with history.Inventory(directory, batch): pass

    def test_changed_history_at_final_output_refuses_cached_page(self):
        with setup() as (directory, batch):
            batch.close = lambda: None
            _, path = save(directory, batch, phase=None)
            original = self.module.encoded
            count = []
            def render(operation, state=None, error=False):
                raw = original(operation, state, error)
                if operation == 'inventory' and state is not None:
                    count.append(1)
                    if len(count) == 2: path.write_bytes(path.read_bytes()[:-1] + b' ')
                return raw
            with patch.object(self.module, 'encoded', side_effect=render):
                code, outputs, _ = self.cli(directory, batch)
            self.assertEqual((code, len(outputs)), (2, 1))
            self.assertFalse(outputs[0]['ok'])
            self.assertIsNone(outputs[0]['result'])

    def test_unknown_prior_effect_blocks_fresh_confirmation_before_new_scope_or_native_start(self):
        for phase in (None, 'source_lost'):
            with self.subTest(phase=phase), setup() as (directory, batch), Peer() as peer:
                relocate(batch, peer.address)
                save(directory, batch, phase=phase)
                before = bytes_in(directory)
                with authority_for(self.module, batch) as authority, \
                     patch.object(self.module, '_frontier', side_effect=frontier):
                    token = self.module.review_progress(batch, authority, 20, 5000)['progress_review']['confirm_progress']
                    batch.budget = Budget()
                    with self.assertRaises(ValueError):
                        self.module.progress(batch, authority, 20, 5000, token, str(directory))
                self.assertNotIn('PrepareRun', peer.calls)
                self.assertNotIn('CommitRun', peer.calls)
                self.assertEqual(before, bytes_in(directory))

    def test_resolved_prior_history_allows_one_actual_clock_commit_and_retains_its_association(self):
        with setup() as (directory, batch), Peer() as peer:
            relocate(batch, peer.address)
            save(directory, batch)
            with authority_for(self.module, batch) as authority, \
                 patch.object(self.module, '_frontier', side_effect=frontier):
                reviewed = self.module.review_progress(batch, authority, 20, 5000)
                token = reviewed['progress_review']['confirm_progress']
                batch.budget = Budget()
                out = self.module.progress(batch, authority, 20, 5000, token, str(directory))
                self.assertEqual(peer.calls.count('PrepareRun'), 1)
                self.assertEqual(peer.calls.count('CommitRun'), 1)
                self.assertTrue(out['construction_progress']['clock_commit_invoked_this_call'])
                self.assertEqual(len(out['construction_progress']['prior_clock_inventory_digest']), 64)
                # The injected old-tick route is not silently treated as post-stop readiness.
                self.assertFalse(out['construction_progress']['next_step_ready'])
                self.assertFalse(out['construction_progress']['construction_completion_proven'])
            batch.budget = Budget()
            with history.Inventory(directory, batch) as owner:
                self.assertTrue(owner.page()['directory_history_reconciled'])
                self.assertEqual(owner.page()['counts']['records'], 2)

    def test_recovery_of_discovered_original_key_clears_only_its_real_terminal_outcome(self):
        with setup() as (directory, batch), Peer() as peer:
            relocate(batch, peer.address)
            prior, _ = save(directory, batch, phase=None)
            peer.records[prior.intent()['idempotency_key']] = (prior.intent(), 'stopped')
            with history.Inventory(directory, batch) as owner:
                page = owner.page(selection='pending')
                token = page['rows'][0]['clock_record_id']
            with authority_for(self.module, batch) as authority:
                out = self.module.query_progress(batch, authority, token, str(directory))
            self.assertTrue(out['construction_progress']['native_query_contacted'])
            self.assertEqual(peer.calls.count('QueryRun'), 1)
            self.assertNotIn('PrepareRun', peer.calls); self.assertNotIn('CommitRun', peer.calls)
            with history.Inventory(directory, batch) as owner:
                self.assertTrue(owner.page()['directory_history_reconciled'])
            with patch.object(socket, 'socket', side_effect=AssertionError('terminal offline')):
                self.module.query_progress(batch, None, token, str(directory))

    def test_lost_commit_reply_is_discovered_and_query_never_redispatches(self):
        with setup() as (directory, batch), Peer() as peer:
            relocate(batch, peer.address); peer.drop_commit_reply = True
            with authority_for(self.module, batch) as authority, \
                 patch.object(self.module, '_frontier', side_effect=frontier):
                token = self.module.review_progress(batch, authority, 20, 5000)['progress_review']['confirm_progress']
                batch.budget = Budget()
                with self.assertRaises((ValueError, OSError)):
                    self.module.progress(batch, authority, 20, 5000, token, str(directory))
                with history.Inventory(directory, batch) as owner:
                    page = owner.page()
                self.assertTrue(page['rows'][0]['query_supported'])
                out = self.module.query_progress(batch, authority, page['rows'][0]['clock_record_id'], str(directory))
                self.assertEqual(out['construction_progress']['clock_result']['record']['phase'], 'stopped')
            self.assertEqual(peer.calls.count('CommitRun'), 1)
            self.assertEqual(peer.calls.count('QueryRun'), 1)

    def test_prior_file_replacement_after_prepare_ack_prevents_commit(self):
        with setup() as (directory, batch), Peer() as peer:
            relocate(batch, peer.address)
            _, prior = save(directory, batch)
            def change():
                temporary = directory / 'temporary-replacement'
                temporary.write_bytes(prior.read_bytes()); temporary.chmod(0o600)
                temporary.replace(prior)
            peer.after_prepare = change
            with authority_for(self.module, batch) as authority, \
                 patch.object(self.module, '_frontier', side_effect=frontier):
                token = self.module.review_progress(batch, authority, 20, 5000)['progress_review']['confirm_progress']
                batch.budget = Budget()
                with self.assertRaises(ValueError):
                    self.module.progress(batch, authority, 20, 5000, token, str(directory))
            self.assertEqual(peer.calls.count('PrepareRun'), 1)
            self.assertEqual(peer.calls.count('CommitRun'), 0)
            self.assertTrue((directory / ('progress-' + token + '.run')).exists())

    def test_prior_history_changed_after_intent_sync_prevents_prepare(self):
        with setup() as (directory, batch), Peer() as peer:
            relocate(batch, peer.address)
            _, prior = save(directory, batch)
            original = clock.capsule
            @contextmanager
            def change_after_sync(path, new_intent=None):
                with original(path, new_intent) as intent:
                    prior.write_bytes(prior.read_bytes()[:-1] + b' ')
                    yield intent
            with authority_for(self.module, batch) as authority, \
                 patch.object(self.module, '_frontier', side_effect=frontier):
                token = self.module.review_progress(batch, authority, 20, 5000)['progress_review']['confirm_progress']
                batch.budget = Budget()
                with patch.object(clock, 'capsule', change_after_sync), self.assertRaises(ValueError):
                    self.module.progress(batch, authority, 20, 5000, token, str(directory))
            self.assertNotIn('PrepareRun', peer.calls); self.assertNotIn('CommitRun', peer.calls)
            self.assertTrue((directory / ('progress-' + token + '.run')).exists())

    def test_clock_directory_gate_is_not_weakened_by_a_terminal_only_inventory_page(self):
        with setup() as (directory, batch), Peer() as peer:
            relocate(batch, peer.address); save(directory, batch, phase=None)
            batch.close = lambda: None
            code, output, _ = self.cli(directory, batch, '--selection', 'terminal')
            self.assertEqual(code, 0)
            self.assertEqual(output[0]['result']['clock_inventory']['rows'], [])
            self.assertEqual(output[0]['result']['clock_inventory']['counts']['unresolved'], 1)
            with authority_for(self.module, batch) as authority, \
                 patch.object(self.module, '_frontier', side_effect=frontier):
                token = self.module.review_progress(batch, authority, 20, 5000)['progress_review']['confirm_progress']
                batch.budget = Budget()
                with self.assertRaises(ValueError): self.module.progress(batch, authority, 20, 5000, token, str(directory))
            self.assertNotIn('CommitRun', peer.calls)


if __name__ == '__main__':
    unittest.main()
