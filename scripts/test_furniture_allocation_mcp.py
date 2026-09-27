#!/usr/bin/env python3
"""Exercise compiled Rust furniture allocation through modern MCP and native TCP.

Pass the actual dfmcp-live-operations-paged-dev-server executable with --binary.
An independent joined operations/1.4 peer supplies complete native-layout bytes;
Rust performs paging, canonical publication, allocation and Agent Turn rendering.
No substitute implementation or success-by-skipping is used when Rust is absent.
This is process/native-wire development evidence, not DFHack or live-game admission.
Beads: df-dfhack-bridge-plane-c-pic.3 / df-dfhack-bridge-plane-c-pic.4.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import threading
import unittest

import furniture_allocation as allocation
from furniture_plan import FurniturePlan, canonical
import test_build_placement_mcp as stdio
from test_construction_monitor_rpc import read_message, wire_message
from test_construction_receipt import U, I, TEXT, REF, TICK, job, operations

SECRET = stdio.SECRET
EXECUTABLE: Path | None = None


def item(identity, kind='BED', *, x=10, y=10, z=0, material=0, material_index=1,
         subtype=-1, flags=64, stack=1, container=None):
    native_type = {'BED': 101, 'CHAIR': 102, 'TABLE': 103, 'BOX': 104, 'BAR': 105}[kind]
    return (U(identity) + I(native_type) + TEXT(kind) + I(subtype, material, material_index)
            + U(stack) + I(x, y, z) + U(flags) + REF(container) + REF(None))


def inventory(tick=TICK + 1, *, missing=()):
    rows = {
        100: item(100),
        101: item(101, x=20, material_index=2),
        102: item(102, 'CHAIR', x=12, material_index=3),
        103: item(103, 'TABLE', x=13, material_index=4),
        104: item(104, flags=65),                         # Forbidden, despite a good position.
        105: item(105),                                   # Attached to an observed job.
        106: item(106, container=107),                     # Direct-ground position is not enough.
        107: item(107, 'BOX'),                             # Unsupported furniture type.
        108: item(108, 'TABLE', stack=2),                   # Not an exact singleton.
        109: item(109, material_index=2),                  # Explicitly excluded by the request.
    }
    return operations(tick, jobs=(job(200, kind='Other', holder=None, count=1),),
                      buildings=(), items=tuple(raw for identity, raw in sorted(rows.items()) if identity not in missing),
                      attachments=((200, 105, 0, -1),), horizons=(201, 0, 110))


def slot(name, kind, x, **constraints):
    return {'name': name, 'kind': kind, 'target': [x, 10, 0], **constraints}


def request(*, shortage=False):
    slots = [slot('d-table', 'table', 13, after=['c-chair', 'a-generic']),
             slot('b-special', 'bed', 11, material=[0, 1]),
             slot('a-generic', 'bed', 10, **({'material': [0, 1]} if shortage else {})),
             slot('c-chair', 'chair', 12)]
    return {'schema': 'dfmcp.query/1', 'query': {
        'kind': 'furniture_allocation', 'world_folder': 'region1', 'site': 2,
        'slots': slots, 'excluded_items': [109],
    }}


class NativePeer:
    """Only Handshake/ReadObservation, one native connection, immutable pages."""
    def __init__(self, snapshots, *, bad_release_at=None):
        self.snapshots = tuple(snapshots)
        self.bad_release_at = bad_release_at
        self.connections = self.captures = self.pages = self.releases = 0
        self.bindings, self.calls = [], []
        self.errors = []
        self.closed = threading.Event()
        self.server = socket.socket()
        self.server.bind(('127.0.0.1', 0))
        self.server.listen(2)
        self.server.settimeout(0.05)
        self.address = self.server.getsockname()
        self.thread = threading.Thread(target=self.serve, daemon=False)

    @staticmethod
    def read(sock, count):
        data = bytearray()
        while len(data) < count:
            part = sock.recv(count - len(data))
            if not part:
                raise EOFError()
            data += part
        return bytes(data)

    @staticmethod
    def send(sock, data):
        for start in range(0, len(data), 503):
            sock.sendall(data[start:start + 503])

    def serve(self):
        while not self.closed.is_set():
            try:
                sock, _ = self.server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self.connections += 1
            with sock:
                sock.settimeout(10)
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                try:
                    assert self.connections == 1, 'unexpected native reconnect or second session'
                    self.connection(sock)
                except (EOFError, ConnectionError):
                    pass
                except BaseException as error:
                    self.errors.append(error)

    def connection(self, sock):
        assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
        self.send(sock, b'DFHack!\n' + struct.pack('<i', 1))
        methods, limits, nonce, current, offset = {}, None, None, None, 0
        while not self.closed.is_set():
            method, size = struct.unpack('<h2xi', self.read(sock, 8))
            assert 0 <= size <= 2048
            fields = read_message(self.read(sock, size))
            if method == 0:
                assert set(fields) == {1, 2, 3, 4}
                name = fields[1].decode('ascii')
                assert name in ('Handshake', 'ReadObservation'), 'effectful native method bound'
                assert fields[2] == b'dfmcp.operations.v1_4.Request'
                assert fields[3] == b'dfmcp.operations.v1_4.Reply'
                assert fields[4] == b'dfmcp_operations_v1_4'
                assert name not in methods.values()
                identifier = len(methods) + 2
                methods[identifier] = name
                self.bindings.append(name)
                response = {1: identifier}
            else:
                name = methods[method]
                self.calls.append(name)
                assert fields[1] == SECRET and 16 <= len(fields[2]) <= 64
                assert fields[3] == 1 and fields[4] == 4
                assert set(fields) == (set(range(1, 9)) | {10, 11, 12}
                                       | ({9} if fields.get(9) else set()))
                actual_limits = tuple(fields[index] for index in (5, 6, 7, 8, 11))
                if limits is None:
                    limits, nonce = actual_limits, fields[2]
                assert actual_limits == limits and fields[2] == nonce
                assert 1 <= limits[0] <= 4096 and 1 <= limits[1] <= 4096
                assert 1 <= limits[2] <= 65536 and 1024 <= limits[3] <= 16 * 1024 * 1024
                assert 16384 <= limits[4] <= 262144
                response = {1: 1, 2: 0, 3: nonce, 4: 1, 5: 4,
                            6: 7, 7: b'allocation-test-df', 8: b'allocation-test-dfhack'}
                token, requested_offset, release = fields.get(9, b''), fields[10], fields[12]
                if name == 'Handshake':
                    assert not token and requested_offset == 0 and release == 0 and current is None
                elif release:
                    assert release == 1 and current is not None and token == self.captures.to_bytes(16, 'big')
                    assert requested_offset == 0 and offset == len(current)
                    self.releases += 1
                    response[10] = (b'x' * 16 if self.captures == self.bad_release_at else token)
                    current = None
                else:
                    if not token:
                        assert current is None and requested_offset == 0
                        assert self.captures < len(self.snapshots), 'unrequested extra native observation'
                        current = self.snapshots[self.captures]
                        self.captures += 1
                        offset = 0
                    else:
                        assert current is not None and token == self.captures.to_bytes(16, 'big')
                    assert requested_offset == offset and len(current) <= limits[3]
                    part = current[offset:offset + limits[4]]
                    response.update({9: part, 10: self.captures.to_bytes(16, 'big'), 11: offset,
                                     12: len(current), 13: hashlib.sha256(current).digest(),
                                     14: int(offset + len(part) == len(current))})
                    offset += len(part)
                    self.pages += 1
            encoded = wire_message(response)
            self.send(sock, struct.pack('<h2xi', -1, len(encoded)) + encoded)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, kind, error, trace):
        self.closed.set()
        self.server.close()
        self.thread.join(12)
        if self.thread.is_alive():
            raise AssertionError('native test peer did not quiesce')
        if self.errors:
            if error is not None:
                error.add_note(f'Native peer also failed: {self.errors[0]!r}')
            else:
                raise AssertionError('native test peer failed') from self.errors[0]


def environment(peer):
    values = {key: value for key, value in os.environ.items() if not key.startswith('DFMCP_')}
    values.update({'DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_4': '1',
                   'DFMCP_OPERATIONS_PAGED_ENDPOINT': f'{peer.address[0]}:{peer.address[1]}',
                   'DFMCP_OPERATIONS_PAGED_TOKEN': SECRET.decode()})
    return values


class Client(stdio.Stdio):
    """Reuse the already executed modern JSON-RPC transport, retain world anchors."""
    def tools(self):
        discover = self.request('server/discover')
        assert discover['supportedVersions'] == ['2026-07-28'], discover
        listing = self.request('tools/list')
        assert {value['name'] for value in listing['tools']} == {
            name.replace('.', '_') for name in stdio.TOOLS
        }, listing
        return listing

    def call(self, tool, **arguments):
        result = self.request('tools/call', {'name': 'fortress_' + tool, 'arguments': arguments})
        texts = [value['text'] for value in result['content'] if value.get('type') == 'text']
        assert len(texts) == 1, result
        packet = json.loads(texts[0])
        assert packet['agent_turn']['schema'] == 'dfmcp.agent_turn/1', packet
        assert 'active_work' in packet['agent_turn'], packet
        self.packet_bytes = len(texts[0].encode('utf-8'))
        return packet


class FurnitureAllocationMcpTests(unittest.TestCase):
    def open(self, client, *, output_tokens=16384):
        value = client.call('open_session', page_bytes=16384, max_bytes=1048576,
                            max_items=4096, max_output_tokens=output_tokens, max_wall_millis=30000)
        self.assertTrue(value['ok'], value)
        self.assertEqual(value['agent_turn']['briefing']['bridge_protocol'], '1.4')
        self.assertEqual(value['agent_turn']['anchor'], value['anchor'])
        self.assertIsNotNone(value['anchor'])
        self.assertFalse(value['agent_turn']['briefing']['runtime_admitted'])
        self.assertEqual(value['granted_capabilities'], ['observe', 'query', 'doctor'])
        return value['session_id'], value['anchor']

    def query(self, client, session, requested):
        value = client.call('query', session_id=session, query=requested)
        if value['ok']:
            submitted = requested['query']
            expected = allocation.Request.from_json({
                'schema': 'dfmcp.furniture-request/1',
                **{key: submitted[key] for key in ('world_folder', 'site', 'slots', 'excluded_items')
                   if key in submitted},
            })
            self.assertEqual(value['request'], expected.json())
        return value

    def allocation(self, value, count=4):
        self.assertTrue(value['ok'], value)
        self.assertEqual(value['status'], 'allocated')
        self.assertEqual(value['summary']['maximum_assignable'], count)
        self.assertEqual(len(value['plan']['steps']), count)
        self.assertEqual(len(value['assignments']), count)
        self.assertEqual(value['anchor'], value['agent_turn']['anchor'])
        self.assertFalse(value['truncated'])
        self.assertIsNone(value['continuation'])
        for flag in ('mutation_authority', 'reservation_created', 'commit_compatible',
                     'placement_eligibility_proven', 'items_reserved', 'game_effect_performed', 'production_admitted'):
            self.assertFalse(value[flag], flag)
        artifact = FurniturePlan.from_json(value['plan'])
        self.assertEqual(artifact.digest, value['plan_digest'])
        self.assertEqual(allocation.Request.decode(canonical(value['request'])).digest, value['request_digest'])
        selected = {row['slot']: row['item'] for row in value['assignments']}
        self.assertEqual(value['plan']['steps'], [
            {'name': row['name'], 'kind': row['kind'], 'target': row['target'],
             'after': row['after'], 'item': selected[row['name']]}
            for row in value['request']['slots']
        ])

    def assert_native_reads(self, peer, captures):
        self.assertEqual(peer.connections, 1)
        self.assertEqual(peer.bindings, ['Handshake', 'ReadObservation'])
        self.assertEqual(peer.calls.count('Handshake'), 1)
        self.assertEqual(peer.captures, captures)
        self.assertEqual(peer.releases, captures)
        self.assertEqual(peer.calls.count('ReadObservation'), peer.pages + peer.releases)

    def test_real_modern_stdio_global_assignment_and_complete_inventory_policy(self):
        with NativePeer([inventory()]) as peer, Client(environment(peer)) as client:
            client.tools()
            session, anchor = self.open(client)
            schema = client.call('query', session_id=session, mode='schema')
            self.assertTrue(schema['ok'], schema)
            variants = schema['query_schema']['$defs']['query']['oneOf']
            advertised = next(value for value in variants
                              if value['properties']['kind'].get('const') == 'furniture_allocation')
            self.assertEqual(advertised['properties']['slots']['maxItems'], 32)
            requested = request()
            requested['expected_anchor'] = anchor
            value = self.query(client, session, requested)
            self.allocation(value)
            self.assertEqual(value['total_distance'], 11)
            self.assertEqual([row['item'] for row in value['assignments']], [101, 100, 102, 103])
            self.assertEqual(value['summary']['observed_items'], 10)
            self.assertEqual(value['summary']['candidate_items'], 4)
            self.assertEqual(value['summary']['item_counts_by_primary_policy_reason'], {
                'candidate_furniture': 4, 'not_exclusively_ground_flags': 1, 'job_attached_item': 1,
                'contained_item': 1, 'unsupported_furniture_type': 1, 'not_singleton': 1,
                'explicitly_excluded_item': 1,
            })
            self.assertEqual(value['source'], {'world_folder': 'region1', 'site': 2, 'bridge_generation': 7,
                                               'df_version': 'allocation-test-df', 'dfhack_version': 'allocation-test-dfhack'})
            self.assertEqual(value['plan']['steps'][-1]['after'], ['a-generic', 'c-chair'])
            for row in value['assignments']:
                self.assertEqual(row['item_handle']['entity_id'], str((2 << 40) + row['item']))
                self.assertEqual(row['item_handle']['generation'], 1)
            reordered = deepcopy(requested)
            reordered['query']['slots'].reverse()
            again = self.query(client, session, reordered)
            for key in ('request_digest', 'analysis_digest', 'plan_digest', 'plan', 'assignments', 'source_digest'):
                self.assertEqual(again[key], value[key], key)
            denied = client.call('commit', session_id=session)
            self.assertFalse(denied['ok'])
            self.assertEqual(denied['error']['code'], 'capability_denied')
            self.assert_native_reads(peer, 1)

    def test_joint_shortage_emits_full_witness_without_partial_executable_plan(self):
        with NativePeer([inventory()]) as peer, Client(environment(peer)) as client:
            session, _ = self.open(client)
            value = self.query(client, session, request(shortage=True))
            self.assertTrue(value['ok'], value)
            self.assertEqual(value['status'], 'shortage')
            self.assertEqual(value['summary']['maximum_assignable'], 3)
            self.assertIsNone(value['plan'])
            self.assertIsNone(value['plan_digest'])
            self.assertEqual(value['assignments'], [])
            self.assertEqual(value['shortage']['slots'], ['a-generic', 'b-special'])
            self.assertEqual(value['shortage']['candidate_items'], [100])
            self.assertEqual(value['shortage']['missing'], 1)
            self.assertEqual(value['shortage']['candidate_evidence'][0]['item'], 100)
            self.allocation(self.query(client, session, request()))
            for field, changed in (('world_folder', 'other-fortress'), ('site', 3), ('limit', 1)):
                wrong = request()
                wrong['query'][field] = changed
                refused = self.query(client, session, wrong)
                self.assertFalse(refused['ok'], refused)
                self.assertNotIn('plan', refused)
            self.assert_native_reads(peer, 1)

    def test_only_explicit_refresh_changes_supply_and_returned_item_generation(self):
        frames = [inventory(), inventory(TICK + 2, missing=(100,)), inventory(TICK + 3)]
        with NativePeer(frames) as peer, Client(environment(peer)) as client:
            session, anchor = self.open(client)
            initial = self.query(client, session, request())
            self.allocation(initial)
            observed = client.call('observe', session_id=session)
            self.assertTrue(observed['ok'], observed)
            self.assertNotEqual(observed['anchor'], anchor)
            stale = request()
            stale['expected_anchor'] = anchor
            refused = self.query(client, session, stale)
            self.assertFalse(refused['ok'])
            self.assertEqual(refused['error']['code'], 'stale_anchor')
            reduced = self.query(client, session, request())
            self.assertEqual(reduced['status'], 'shortage')
            self.assertIsNone(reduced['plan'])
            self.assertEqual(peer.captures, 2)
            restored = client.call('observe', session_id=session)
            self.assertTrue(restored['ok'], restored)
            current = self.query(client, session, request())
            self.allocation(current)
            self.assertEqual(current['plan_digest'], initial['plan_digest'])
            self.assertNotEqual(current['source_digest'], initial['source_digest'])
            self.assertNotEqual(current['analysis_digest'], initial['analysis_digest'])
            returned = next(row for row in current['assignments'] if row['item'] == 100)
            self.assertEqual(returned['item_handle']['generation'], 2)
            self.assert_native_reads(peer, 3)

    def test_failed_release_preserves_prior_anchor_and_fences_further_allocation(self):
        with NativePeer([inventory(), inventory(TICK + 2, missing=(100,))], bad_release_at=2) as peer, \
                Client(environment(peer)) as client:
            session, anchor = self.open(client)
            self.allocation(self.query(client, session, request()))
            refused = client.call('observe', session_id=session)
            self.assertFalse(refused['ok'], refused)
            self.assertEqual(refused['agent_turn']['anchor'], anchor)
            self.assertEqual(refused['agent_turn']['continuity']['status'], 'stale')
            calls = list(peer.calls)
            failed = self.query(client, session, request())
            self.assertFalse(failed['ok'], failed)
            self.assertEqual(failed['error']['code'], 'adapter_unavailable')
            self.assertNotIn('plan', failed)
            self.assertEqual(peer.calls, calls)
            self.assert_native_reads(peer, 2)

    def test_complete_32_slot_result_uses_verified_pages_and_refuses_small_output_budget(self):
        kinds = ('BED', 'CHAIR', 'TABLE')
        chosen = tuple(item(100 + index, kinds[index % 3], x=10 + index) for index in range(32))
        extras = tuple(item(index, 'BAR') for index in range(132, 2100))
        raw = operations(buildings=(), items=chosen + extras, horizons=(0, 0, 2100))
        self.assertGreater(len(raw), 16384)
        requested = {'schema': 'dfmcp.query/1', 'query': {
            'kind': 'furniture_allocation', 'world_folder': 'region1', 'site': 2,
            'slots': [slot(f'furniture-{index:02}', kinds[index % 3].lower(), 10 + index)
                      for index in range(32)],
        }}
        for output_tokens in (16384, 2048):
            with self.subTest(output_tokens=output_tokens), NativePeer([raw]) as peer, Client(environment(peer)) as client:
                session, _ = self.open(client, output_tokens=output_tokens)
                value = self.query(client, session, requested)
                if output_tokens == 16384:
                    self.allocation(value, 32)
                    self.assertEqual(value['summary']['observed_items'], 2000)
                    self.assertEqual(value['summary']['candidate_items'], 32)
                    self.assertEqual([row['item'] for row in value['assignments']], list(range(100, 132)))
                    self.assertLessEqual(client.packet_bytes, 65536)
                else:
                    self.assertFalse(value['ok'], value)
                    self.assertEqual(value['error']['code'], 'budget_exceeded')
                    self.assertNotIn('plan', value)
                    self.assertLessEqual(client.packet_bytes, 8192)
                    small = deepcopy(requested)
                    small['query']['slots'] = small['query']['slots'][:1]
                    self.allocation(self.query(client, session, small), 1)
                self.assertGreater(peer.pages, 1)
                self.assert_native_reads(peer, 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args, remaining = parser.parse_known_args()
    global EXECUTABLE
    EXECUTABLE = args.binary.resolve(strict=True)
    if not EXECUTABLE.is_file() or not os.access(EXECUTABLE, os.X_OK):
        parser.error('--binary must name an executable regular file')
    stdio.EXECUTABLE = EXECUTABLE
    unittest.main(argv=[__file__, *remaining])


if __name__ == '__main__':
    main()
