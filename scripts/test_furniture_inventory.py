"""Actual allocator/decoder/transport/CLI tests over independently encoded native bytes.

The joined loopback peer implements only the existing operations/1.4 read wire
contract. It is a test double, not a DFHack build or live fortress qualification.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import allocate_furniture as cli
import furniture_inventory as f
from furniture_allocation import Request, Slot
from furniture_plan import FurniturePlan, canonical

ROOT = Path(__file__).resolve().parents[1]
TOKEN = 'test-only-operations-token-' + 'q' * 32
PAGE = 65536
MEASUREMENTS = {'largest_complete_response_bytes': 0, 'largest_capture_bytes': 0, 'maximum_pages': 0}
U = lambda *values: struct.pack('>' + 'I' * len(values), *values)
I = lambda *values: struct.pack('>' + 'i' * len(values), *values)
TEXT = lambda value: struct.pack('>H', len(value.encode())) + value.encode()
REF = lambda value: b'\0' if value is None else b'\1' + U(value)


def item(identity=42, kind='BED', native_type=None, position=(10, 10, 2), flags=64,
         container=None, holder=None, material=419, material_index=-1, subtype=-1, stack=1):
    if native_type is None:
        native_type = {'BED': 101, 'CHAIR': 102, 'TABLE': 103}.get(kind, 104)
    return (U(identity) + I(native_type) + TEXT(kind) + I(subtype, material, material_index)
            + U(stack) + I(*position) + U(flags) + REF(container) + REF(holder))


def job(identity=90, count=0, holder=None, filters=0, kind='Other'):
    return (U(identity) + I(1) + TEXT(kind) + TEXT('') + b'\0\0' + I(10, 10, 2)
            + REF(None) + REF(holder) + I(-1) + U(count, filters))


def building(identity=70):
    return U(identity) + I(1) + TEXT('Bed') + I(15, 15, 15, 15, 2, 1, 1)


def capture(items=None, jobs=(), buildings=(), attachments=(), folder='region1', site=2,
            tick=806500, horizons=(10000, 10000, 2147483647)):
    items = (item(),) if items is None else items
    year, annual = divmod(tick, 403200)
    jp = (b'DFMJ1200' + U(year, annual) + b'\1' + I(site) + U(horizons[0])
          + TEXT(folder) + U(len(jobs)) + b''.join(jobs))
    return (b'DFMO1400' + U(len(jp)) + jp + U(*horizons[1:], len(buildings)) + b''.join(buildings)
            + U(len(items)) + b''.join(items) + U(len(attachments)) + b''.join(I(*v) for v in attachments))


def request(slots=None, **kw):
    return Request(kw.get('folder', 'region1'), kw.get('site', 2),
                   tuple(slots or [Slot('bed', 'bed', (15, 15, 2))]), tuple(kw.get('excluded', ())))


def environment(address):
    env = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    env.update({f.OPT_IN: '1', f.ENDPOINT: f'{address[0]}:{address[1]}',
                f.OPERATIONS_TOKEN: TOKEN, 'PYTHONDONTWRITEBYTECODE': '1'})
    return env


# Independent small protobuf writer/reader: not production encode/decode calls.
def vint(value):
    out = bytearray()
    while value > 127:
        out.append((value & 127) | 128)
        value >>= 7
    return bytes(out) + bytes([value])


def pb(fields):
    raw = b''
    for key, value in sorted(fields.items()):
        if isinstance(value, bytes):
            raw += vint(key * 8 + 2) + vint(len(value)) + value
        else:
            raw += vint(key * 8) + vint(value)
    return raw


def unpb(raw):
    values, index = {}, 0
    def number():
        nonlocal index
        value, shift = 0, 0
        while True:
            byte = raw[index]
            index += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
            assert shift < 70
    while index < len(raw):
        tag = number()
        assert tag >> 3 not in values
        if tag & 7 == 0:
            value = number()
        else:
            assert tag & 7 == 2
            size = number()
            value = raw[index:index + size]
            assert len(value) == size
            index += size
        values[tag >> 3] = value
    return values


class Peer:
    def __init__(self, raw=None, fault=None, version=None):
        self.raw = capture() if raw is None else raw
        self.fault, self.version = fault, version or ('test-df', 'test-dfhack')
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.listener.settimeout(2)
        self.address = self.listener.getsockname()
        self.connection = None
        self.thread = threading.Thread(target=self.serve)
        self.bindings, self.calls, self.errors = [], [], []
        self.stopping, self.page_number, self.released = False, 0, False

    def exact(self, size):
        raw = b''
        while len(raw) < size:
            part = self.connection.recv(size - len(raw))
            if not part:
                raise EOFError()
            raw += part
        return raw

    def send(self, raw):
        # Deliberately fragment both header and payload boundaries.
        for part in (raw[:1], raw[1:7], raw[7:]):
            if part:
                self.connection.sendall(part)

    def reply(self, fields, phase):
        if self.fault == 'notifications' and phase == 'handshake':
            self.send(struct.pack('<h2xi', -3, 8) + b'not-data')
        if self.fault == 'notification_count' and phase == 'page':
            for _ in range(9):
                self.send(struct.pack('<h2xi', -3, 1) + b'x')
        if self.fault == 'notification_bytes' and phase == 'page':
            for _ in range(5):
                self.send(struct.pack('<h2xi', -3, 65536) + b'x' * 65536)
        if self.fault == 'notification_total':
            for _ in range(4):
                self.send(struct.pack('<h2xi', -3, 65536) + b'x' * 65536)
        raw = pb(fields)
        if self.fault == 'duplicate_field' and phase == 'page':
            raw += b'\x08\x01'
        if self.fault == 'unknown_field' and phase == 'page':
            raw += vint(15 * 8) + b'\0'
        if self.fault == 'nonminimal_varint' and phase == 'page':
            raw = b'\x88\x00' + raw[1:]
        self.send(struct.pack('<h2xi', -1, len(raw)) + raw)

    def serve(self):
        try:
            self.connection, _ = self.listener.accept()
            # Allocation after the verified release is CPU work, not another RPC.
            # Keep the owned peer available until its caller closes; __exit__
            # still shuts down and joins it even on client failure.
            self.connection.settimeout(12)
            self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            assert self.exact(12) == b'DFHack?\n' + struct.pack('<i', 1)
            if self.fault == 'drop_greeting':
                return
            self.send((b'badhead!' if self.fault == 'greeting' else b'DFHack!\n') + struct.pack('<i', 1))
            while True:
                method, size = struct.unpack('<h2xi', self.exact(8))
                assert 0 <= size <= 2048
                fields = unpb(self.exact(size))
                if method == 0:
                    name = fields[1].decode()
                    assert name == ('Handshake', 'ReadObservation')[len(self.bindings)]
                    assert fields == {1: name.encode(), 2: b'dfmcp.operations.v1_4.Request',
                                      3: b'dfmcp.operations.v1_4.Reply', 4: b'dfmcp_operations_v1_4'}
                    self.bindings.append(name)
                    identity = 2 if self.fault == 'alias' else len(self.bindings) + 1
                    self.reply({1: identity}, 'binding')
                    continue
                assert method in (2, 3), 'unexpected method, including any native writer'
                assert fields[1] == TOKEN.encode() and len(fields[2]) == 32
                assert {i: fields[i] for i in (3, 4, 5, 6, 7, 8, 11)} == {
                    3: 1, 4: 4, 5: 4096, 6: 4096, 7: 65536, 8: 16 * 1024 * 1024, 11: 65536}
                phase = 'handshake' if method == 2 else 'release' if fields[12] else 'page'
                self.calls.append(phase)
                reply = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 4, 6: 987,
                         7: self.version[0].encode(), 8: self.version[1].encode()}
                if method == 2:
                    assert set(fields) == set(range(1, 9)) | {11}
                    if self.fault == 'nonce': reply[3] = b'x' * 32
                    if self.fault == 'minor': reply[5] = 5
                    if self.fault == 'refusal': reply[1], reply[2] = 0, 1
                    if self.fault == 'generation_zero': reply[6] = 0
                    self.reply(reply, phase)
                    continue
                assert set(fields) == set(range(1, 13))
                token = b'capture-token-14'
                assert len(token) == 16
                if phase == 'release':
                    assert fields[9] == token and fields[10] == 0 and fields[12] == 1
                    self.released = True
                    if self.fault == 'drop_release': return
                    reply[10] = b'x' * 16 if self.fault == 'release_token' else token
                    if self.fault == 'release_generation': reply[6] += 1
                    if self.fault == 'release_extra': reply[14] = 1
                    self.reply(reply, phase)
                    continue
                self.page_number += 1
                offset = (self.page_number - 1) * PAGE
                assert fields[9] == (b'' if offset == 0 else token) and fields[10] == offset
                assert fields[12] == 0 and offset < len(self.raw)
                chunk = self.raw[offset:offset + PAGE]
                complete = offset + len(chunk) == len(self.raw)
                if self.fault == 'drop_last_page' and complete: return
                if self.fault == 'stall': time.sleep(0.35)
                reply.update({9: chunk, 10: token, 11: offset, 12: len(self.raw),
                              13: hashlib.sha256(self.raw).digest(), 14: int(complete)})
                if self.fault == 'total': reply[12] = 16 * 1024 * 1024 + 1
                if self.fault == 'token_width': reply[10] = b'x' * 15
                if self.fault == 'offset': reply[11] += 1
                if self.fault == 'short_page': reply[9] = chunk[:-1]
                if self.fault == 'complete': reply[14] = int(not complete)
                if self.fault == 'hash': reply[13] = b'x' * 32
                if self.fault == 'wire_type': reply[11] = b''
                if self.page_number > 1:
                    if self.fault == 'source_change': reply[6] += 1
                    if self.fault == 'software_change': reply[7] = b'changed-df'
                    if self.fault == 'token_change': reply[10] = b'x' * 16
                    if self.fault == 'total_change': reply[12] += 1
                    if self.fault == 'hash_change': reply[13] = b'x' * 32
                    if self.fault == 'replay': reply[11] = 0
                self.reply(reply, phase)
        except (EOFError, ConnectionResetError, BrokenPipeError):
            pass
        except OSError as error:
            if not self.stopping:
                self.errors.append(error)
        except BaseException as error:
            self.errors.append(error)
        finally:
            if self.connection is not None:
                self.connection.close()

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stopping = True
        if self.connection is not None:
            try: self.connection.shutdown(socket.SHUT_RDWR)
            except OSError: pass
        self.listener.close()
        self.thread.join(3)
        if self.thread.is_alive():
            raise AssertionError('native test peer was not joined')
        if self.errors:
            raise self.errors[0]


def live(peer, req=None, budget=None):
    with patch.dict(os.environ, environment(peer.address), clear=True):
        raw = f.run(req or request(), f.Authority.load(), budget or f.Budget(10000))
        out = json.loads(raw)
        MEASUREMENTS['largest_complete_response_bytes'] = max(MEASUREMENTS['largest_complete_response_bytes'], len(raw))
        MEASUREMENTS['largest_capture_bytes'] = max(MEASUREMENTS['largest_capture_bytes'], out['result']['source']['capture_bytes'])
        MEASUREMENTS['maximum_pages'] = max(MEASUREMENTS['maximum_pages'], peer.page_number)
        return out


class ProjectionTests(unittest.TestCase):
    def test_all_512_native_flag_words_and_candidate_policy(self):
        for flags in range(512):
            out = f.project(request(), capture(items=(item(flags=flags),)), lambda: None)
            self.assertEqual(out['status'] == 'allocated', flags == 64, flags)
            self.assertEqual(out['projection']['counts']['candidate'], int(flags == 64), flags)

    def test_native_kind_material_subtype_identity_preserved(self):
        r = request([Slot('bed', 'bed', (15, 15, 2), material=(419, -1)),
                     Slot('chair', 'chair', (16, 15, 2), subtype=7), Slot('table', 'table', (17, 15, 2))])
        raw = capture(items=(item(), item(43, 'CHAIR', subtype=7), item(44, 'TABLE')))
        out = f.project(r, raw, lambda: None)
        self.assertEqual(out['status'], 'allocated')
        self.assertEqual([v['item'] for v in out['assignments']], [42, 43, 44])
        self.assertEqual(out['source']['capture_sha256'], hashlib.sha256(raw).hexdigest())
        self.assertEqual(out['source']['game_tick'], 806500)
        self.assertEqual(FurniturePlan.from_json(out['plan']).digest, out['plan_digest'])

    def test_job_attachment_beats_apparently_free_flags(self):
        raw = capture(items=(item(42), item(43)), jobs=(job(count=1),), attachments=((90, 42, 0, -1),))
        out = f.project(request(), raw, lambda: None)
        self.assertEqual(out['assignments'][0]['item'], 43)
        self.assertEqual(out['projection']['counts']['related_item'], 1)
        self.assertEqual(out['projection']['attachments'], 1)

    def test_container_holder_contents_stack_material_position_and_exclusions(self):
        values = (item(0, kind='OTHER'), item(1, container=2), item(2), item(3, holder=70),
                  item(4, stack=2), item(5, material=-1), item(6, position=(-1, 2, 2)),
                  item(7), item(8))
        out = f.project(request(excluded=(7,)), capture(items=values, buildings=(building(),)), lambda: None)
        self.assertEqual(out['assignments'][0]['item'], 8)
        counts = out['projection']['counts']
        self.assertEqual(counts, {'unsupported_kind': 1, 'projected_flags': 0, 'related_item': 3,
                                 'non_singleton': 1, 'unknown_material': 1, 'invalid_position': 1,
                                 'excluded': 1, 'candidate': 1})
        self.assertEqual(sum(counts.values()), len(values))

    def test_whole_roster_malformed_even_if_selected_item_would_be_valid(self):
        corrupt = [capture(items=(item(), item(43, flags=512))),
                   capture(items=(item(), item(43, container=44))),
                   capture(items=(item(), item(43, container=43))),
                   capture(items=(item(), item(43, native_type=101, kind='CHAIR'))),
                   capture(items=(item(), item())),
                   capture(items=(item(), item(43)), jobs=(job(count=1),)),
                   capture(items=(item(), item(43)), jobs=(job(count=1),), attachments=((90, 44, 0, -1),)),
                   capture(items=(item(), item(43)), jobs=(job(count=1),), attachments=((90, 43, 0, 0),))]
        for raw in corrupt:
            with self.subTest(length=len(raw)), self.assertRaises(ValueError):
                f.project(request(), raw, lambda: None)

    def test_every_truncated_prefix_and_trailing_bytes(self):
        raw = capture(items=(item(), item(43)), jobs=(job(count=1),), attachments=((90, 43, 0, -1),))
        for end in range(len(raw)):
            with self.subTest(end=end), self.assertRaises(ValueError):
                f.project(request(), raw[:end], lambda: None)
        with self.assertRaises(ValueError):
            f.project(request(), raw + b'\0', lambda: None)

    def test_foreign_fortress_refuses_instead_of_relabeling_inventory(self):
        for raw in (capture(folder='region2'), capture(site=3)):
            with self.assertRaises(ValueError):
                f.project(request(), raw, lambda: None)

    def test_full_65536_item_native_capture_and_all_32_targets(self):
        r = request([Slot(f'{i:02}', 'bed', (i + 1, 10, 2)) for i in range(32)])
        raw = capture(items=tuple(item(i) for i in range(65536)))
        budget = f.Budget(10000)
        out = f.project(r, raw, budget.work)
        self.assertEqual(out['projection']['items'], 65536)
        self.assertEqual(out['maximum_assignable'], 32)
        self.assertEqual([v['item'] for v in out['assignments']], list(range(32)))
        self.assertLess(len(f.bounded_output(f.packet(out))), f.MAX_OUTPUT)
        self.assertLess(budget.work_steps, 20_000_000)

    def test_output_refusal_instead_of_truncation_or_fabricated_empty_result(self):
        with self.assertRaises(ValueError):
            f.bounded_output({'payload': 'x' * f.MAX_OUTPUT})
        error = json.loads(f.bounded_output(f.packet(None, 'ValueError')))
        self.assertFalse(error['ok'])
        self.assertIsNone(error['result'])
        self.assertEqual(error['agent_turn']['references'], [])
        self.assertEqual(error['agent_turn']['coverage']['operations_capture'], 'unestablished')


class TransportTests(unittest.TestCase):
    def test_real_tcp_two_bindings_complete_release_and_actual_plan(self):
        with Peer(fault='notifications') as peer:
            out = live(peer)
            self.assertEqual(peer.bindings, ['Handshake', 'ReadObservation'])
            self.assertEqual(peer.calls, ['handshake', 'page', 'release'])
            self.assertTrue(peer.released)
            self.assertTrue(out['result']['native_capture_established'])
            self.assertTrue(out['result']['capture_release_verified'])
            self.assertEqual(out['result']['source']['native_generation'], 987)
            self.assertEqual(out['result']['source']['df_version'], 'test-df')
            self.assertIsNone(out['agent_turn']['anchor'])
            self.assertFalse(out['agent_turn']['briefing']['mutation_admissible'])
            self.assertEqual(out['agent_turn']['coverage']['placement_receipts'], 'not_queried')
            self.assertEqual(FurniturePlan.from_json(out['result']['plan']).steps[0].item, 42)

    def test_multipage_inventory_reuses_one_capture_and_one_connection(self):
        raw = capture(items=tuple(item(i) for i in range(2000)))
        self.assertGreater(len(raw), PAGE)
        with Peer(raw) as peer:
            out = live(peer)
            self.assertEqual(peer.page_number, (len(raw) + PAGE - 1) // PAGE)
            self.assertEqual(peer.calls[-1], 'release')
            self.assertEqual(out['result']['projection']['items'], 2000)
            self.assertEqual(out['result']['source']['capture_sha256'], hashlib.sha256(raw).hexdigest())

    def test_full_native_tcp_inventory_with_last_page_job_attachment(self):
        r = request([Slot(f'{i:02}', 'bed', (i + 1, 10, 2)) for i in range(32)])
        raw = capture(items=tuple(item(i) for i in range(65536)), jobs=(job(count=1),),
                      attachments=((90, 0, 0, -1),))
        with Peer(raw) as peer:
            budget = f.Budget(10000)
            out = live(peer, r, budget)
            self.assertEqual(out['result']['projection']['items'], 65536)
            self.assertEqual(out['result']['projection']['counts']['related_item'], 1)
            self.assertEqual([v['item'] for v in out['result']['assignments']], list(range(1, 33)))
            self.assertEqual(peer.page_number, (len(raw) + PAGE - 1) // PAGE)
            self.assertGreater(peer.page_number, 32)
            self.assertEqual(budget.calls, 272 - 2 - 1 - peer.page_number - 1)
            self.assertTrue(peer.released)

    def test_bad_binding_handshake_and_page_shapes_never_return_plan(self):
        faults = ('drop_greeting', 'greeting', 'alias', 'nonce', 'minor', 'refusal', 'generation_zero',
                  'total', 'token_width', 'offset', 'short_page', 'complete', 'hash', 'wire_type',
                  'duplicate_field', 'unknown_field', 'nonminimal_varint', 'drop_last_page')
        for fault in faults:
            with self.subTest(fault=fault), Peer(fault=fault) as peer:
                with self.assertRaises((ValueError, OSError)):
                    live(peer)
                self.assertFalse(peer.released)

    def test_cross_page_source_identity_and_offset_changes_refused(self):
        raw = capture(items=tuple(item(i) for i in range(2000)))
        for fault in ('source_change', 'software_change', 'token_change', 'total_change', 'hash_change', 'replay'):
            with self.subTest(fault=fault), Peer(raw, fault) as peer:
                with self.assertRaises(ValueError):
                    live(peer)
                self.assertEqual(peer.page_number, 2)
                self.assertFalse(peer.released)

    def test_complete_bytes_without_verified_release_never_publish(self):
        for fault in ('drop_release', 'release_token', 'release_generation', 'release_extra'):
            with self.subTest(fault=fault), Peer(fault=fault) as peer:
                with self.assertRaises((ValueError, OSError)):
                    live(peer)
                self.assertTrue(peer.released)

    def test_only_one_acquisition_and_forbidden_profile_has_no_dispatch(self):
        for forbidden in (False, True):
            with Peer() as peer, patch.dict(os.environ, environment(peer.address), clear=True):
                with f.InventoryClient(f.Authority.load(), f.Budget(10000)) as client:
                    if forbidden:
                        with self.assertRaises(ValueError):
                            client._call('build', 'CommitPlacement', {}, set())
                        self.assertTrue(client.closed)
                        self.assertEqual(peer.calls, ['handshake'])
                    else:
                        client.capture_once()
                        before = list(peer.calls)
                        with self.assertRaises(ValueError):
                            client.capture_once()
                        self.assertEqual(peer.calls, before)

    def test_shared_call_network_work_and_real_wall_deadlines(self):
        for name, value in (('calls', 3), ('calls', 4), ('network_bytes', 32), ('work_steps', 1)):
            with self.subTest(name=name, value=value), Peer() as peer:
                budget = f.Budget(10000)
                setattr(budget, name, value)
                with self.assertRaises(ValueError):
                    live(peer, budget=budget)
        with Peer(fault='stall') as peer:
            with self.assertRaises((ValueError, TimeoutError)):
                live(peer, budget=f.Budget(200))

    def test_per_call_and_whole_connection_notification_bounds(self):
        raw = capture(items=tuple(item(i) for i in range(9000)))
        for fault in ('notification_count', 'notification_bytes', 'notification_total'):
            with self.subTest(fault=fault), Peer(raw, fault) as peer:
                with self.assertRaises(ValueError):
                    live(peer)
                self.assertFalse(peer.released)

    def test_revocation_after_native_read_cannot_publish_cached_allocation(self):
        original = f.project
        def revoke(*args):
            result = original(*args)
            os.environ[f.OPT_IN] = '0'
            return result
        with Peer() as peer, patch.object(f, 'project', revoke):
            with self.assertRaises(ValueError):
                live(peer)
            self.assertTrue(peer.released)

    def test_authority_rejects_mutation_credentials_and_changed_endpoint(self):
        env = environment(('127.0.0.1', 5000))
        for key, value in ((f.OPT_IN, '0'), (f.ENDPOINT, 'localhost:5000'),
                           (f.ENDPOINT, '8.8.8.8:5000'), (f.ENDPOINT, '127.0.0.1:05000'),
                           (f.OPERATIONS_TOKEN, 'short'), ('DFMCP_BUILD_TOKEN', 'x' * 64),
                           ('DFMCP_BUILD_ALLOW_PLACE', '1'), ('DFMCP_ADMISSION_TICKET', 'x')):
            with self.subTest(key=key, value=value), patch.dict(os.environ, dict(env, **{key: value}), clear=True):
                with self.assertRaises(ValueError):
                    f.Authority.load()
        with patch.dict(os.environ, env, clear=True):
            authority = f.Authority.load()
            self.assertNotIn(TOKEN, repr(authority))
            os.environ[f.ENDPOINT] = '127.0.0.1:5001'
            with self.assertRaises(ValueError):
                authority.guard()

    def test_native_software_maxima_and_complete_32_slot_output(self):
        names = [f'{i:02}' + 's' * 46 for i in range(32)]
        r = request([Slot(name, 'bed', (i + 1, 10, 2), after=tuple(names[max(0, i - 2):i]))
                     for i, name in enumerate(names)], folder='\U0001f3f0' * 128)
        raw = capture(items=tuple(item(2147483615 + i) for i in range(32)), folder=r.folder)
        with Peer(raw, version=('\U0001f3f0' * 32, '\U0001f3f0' * 32)) as peer:
            output = live(peer, r)
            encoded = f.bounded_output(output)
            self.assertLess(len(encoded), 65536)
            self.assertEqual(len(output['result']['assignments']), 32)
            self.assertEqual(len(FurniturePlan.from_json(output['result']['plan']).steps), 32)
            self.assertEqual(output['result']['request'], r.json())


class CliTests(unittest.TestCase):
    def execute(self, path, env, *extra):
        return subprocess.run([sys.executable, str(ROOT / 'scripts/allocate_furniture.py'),
                               '--request-file', str(path), *extra], env=env, capture_output=True,
                              timeout=12, check=False)

    def test_actual_subprocess_read_allocate_and_export_compatible_plan(self):
        with tempfile.TemporaryDirectory() as directory, Peer() as peer:
            path = Path(directory) / 'request.json'
            original = canonical(request().json())
            path.write_bytes(original)
            response = self.execute(path, environment(peer.address))
            self.assertEqual(response.returncode, 0, response.stderr)
            self.assertEqual(response.stderr, b'')
            out = json.loads(response.stdout)
            self.assertTrue(out['ok'])
            self.assertEqual(out['result']['status'], 'allocated')
            self.assertEqual(FurniturePlan.decode(canonical(out['result']['plan'])).steps[0].item, 42)
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual([v.name for v in Path(directory).iterdir()], ['request.json'])
            self.assertNotIn(TOKEN.encode(), response.stdout)
            self.assertTrue(peer.released)

    def test_subprocess_shortage_is_not_an_empty_successful_plan(self):
        with tempfile.TemporaryDirectory() as directory, Peer(capture(items=())) as peer:
            path = Path(directory) / 'request.json'
            path.write_bytes(canonical(request().json()))
            response = self.execute(path, environment(peer.address))
            self.assertEqual(response.returncode, 0)
            result = json.loads(response.stdout)['result']
            self.assertEqual(result['status'], 'shortage')
            self.assertIsNone(result['plan'])
            self.assertEqual(result['shortage']['missing'], 1)

    def test_subprocess_lost_release_outputs_no_partial_or_secret_facts(self):
        with tempfile.TemporaryDirectory() as directory, Peer(fault='drop_release') as peer:
            path = Path(directory) / 'secret-user-request.json'
            path.write_bytes(canonical(request().json()))
            response = self.execute(path, environment(peer.address))
            self.assertEqual(response.returncode, 2)
            out = json.loads(response.stdout)
            self.assertFalse(out['ok'])
            self.assertIsNone(out['result'])
            self.assertEqual(out['agent_turn']['references'], [])
            for secret in (TOKEN.encode(), str(path).encode(), b'test-df', b'region1'):
                self.assertNotIn(secret, response.stdout)

    def test_bounded_regular_request_file_and_pre_network_refusal(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            good = root / 'good'
            original = canonical(request().json())
            good.write_bytes(original)
            self.assertEqual(cli.read_request(str(good), f.Budget(10000)), request())
            link = root / 'link'
            link.symlink_to(good)
            fifo = root / 'fifo'
            os.mkfifo(fifo)
            for path, data in ((root / 'empty', b''), (root / 'large', b'x' * 16385),
                               (root / 'duplicate', b'{"schema":1,"schema":2}')):
                path.write_bytes(data)
            for path in (link, fifo, root, root / 'empty', root / 'large', root / 'duplicate'):
                with self.subTest(path=path), patch.object(f.socket, 'socket', side_effect=AssertionError('network forbidden')):
                    with self.assertRaises((ValueError, OSError)):
                        cli.read_request(str(path), f.Budget(10000))
            self.assertEqual(good.read_bytes(), original)

    def test_request_rewrite_during_read_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'request'
            raw = canonical(request().json())
            path.write_bytes(raw)
            original_read = os.read
            changed = False
            def rewrite(fd, count):
                nonlocal changed
                result = original_read(fd, count)
                if not changed:
                    path.write_bytes(raw + b' ')
                    changed = True
                return result
            with patch.object(os, 'read', rewrite), self.assertRaises(ValueError):
                cli.read_request(str(path), f.Budget(10000))


if __name__ == '__main__':
    unittest.main()
