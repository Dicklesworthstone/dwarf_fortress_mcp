#!/usr/bin/env python3
"""Actual batch/custody/client/CLI tests with a joined dig/1.16 TCP test peer.

The peer is not a DFHack SDK or live fortress. Native bytes are synthesized from
an explicit deterministic map; all client decoding and file I/O are real.
"""
from contextlib import contextmanager
import hashlib
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
import unittest
from unittest.mock import patch

import dig_blueprint as b
import dig_blueprint_client as c
import dig_designation_client as d
import dig_designation_store as s
from test_dig_blueprint import specification

METRICS = {}


def native_record(intent, state, reason=0):
    before = bytes.fromhex(intent['observation_hex'])
    o = d.observation(before, intent['region'])
    prefix = (b'DFMDGE16' + struct.pack('>QQQ', o['generation'], o['sequence'], o['tick'])
        + d.region_bytes(intent['region']) + bytes([intent['allow_hidden_neighbors']])
        + bytes.fromhex(intent['witness'] + intent['plan_digest'] + intent['prepare_token']))
    known = state == 2
    count = intent['region']['width'] * intent['region']['height'] if known else 0
    after = hashlib.sha256(d.expected_after(before, intent['region'])).digest() if known else bytes(32)
    data = struct.pack('>BBBI', state, reason, known, count) + after
    receipt = d.digest(b'dfmcp-dig-designation-receipt/1', struct.pack('>Q', o['generation'])
        + d.key_bytes(intent['key']) + bytes.fromhex(intent['plan_digest'] + intent['prepare_token']) + data)
    return prefix + data + (receipt if state in (2, 4) else bytes(32)) + d.key_bytes(intent['key'])


class Peer:
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0)); self.listener.listen(8); self.listener.settimeout(.1)
        self.address = f'127.0.0.1:{self.listener.getsockname()[1]}'
        self.source = {'generation': 41, 'df_version': 'df-test', 'dfhack_version': 'dfhack-test'}
        self.sequence, self.tick, self.site, self.folder = 3, 100, 2, 'region1'
        self.dimensions = [64, 64, 16]
        self.designated, self.blocks, self.records = set(), set(), {}
        self.calls, self.errors = [], []
        self.lost_commit = False
        self.maximum_fields = False
        self.bad_effect = False
        self.hidden = set()
        self.hazards = set()
        self.done = threading.Event()
        self.worker = threading.Thread(target=self.serve)

    def __enter__(self):
        self.worker.start(); return self

    def __exit__(self, *_):
        self.done.set(); self.listener.close(); self.worker.join(3)
        if self.worker.is_alive():
            raise AssertionError('native test peer did not join')
        if self.errors:
            raise AssertionError(self.errors)

    def capture(self, region):
        x, y, z, w, h = d.region(region)
        raw = (b'DFMDG016' + struct.pack('>QQQI', self.source['generation'], self.sequence, self.tick, self.site)
            + struct.pack('>III', *self.dimensions) + d.region_bytes(region) + b'\x01'
            + d.text(self.folder.encode()) + struct.pack('>H', (w + 2) * (h + 2) * 3))
        for pz in range(z - 1, z + 2):
            for py in range(y - 1, y + h + 1):
                for px in range(x - 1, x + w + 1):
                    pos = (px, py, pz)
                    if pos in self.hidden:
                        raw += b'\x01'; continue
                    designated = pos in self.designated
                    activated = (px >> 4, py >> 4, pz) in self.blocks
                    values = ((2**32 - 1, 2**32 - 1, 2**32 - 1, 7000, 2**32 - 1,
                               2**32 - 1, 65535, 65535, 7, 15, 31) if self.maximum_fields else
                              (42, 0, 0, 4000 if designated else 0, 0 if activated else 5, 0,
                               10015, 10015, int(designated), int(pos in self.hazards), 1 | (16 if activated else 0)))
                    raw += b'\x02' + struct.pack('>IIIIIIHHBBB', *values)
        return raw

    @staticmethod
    def read(sock, count):
        data = b''
        while len(data) < count:
            part = sock.recv(count - len(data))
            if not part:
                return None
            data += part
        return data

    def serve(self):
        while not self.done.is_set():
            try:
                sock, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                if self.done.is_set():
                    return
                raise
            try:
                with sock:
                    sock.settimeout(3); sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    assert self.read(sock, 12) == b'DFHack?\n' + struct.pack('<i', 1)
                    sock.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                    methods = {}
                    while not self.done.is_set():
                        header = self.read(sock, 8)
                        if header is None:
                            break
                        method, length = struct.unpack('<h2xi', header)
                        assert 0 <= length <= 2048
                        request = d.decode(self.read(sock, length), 14)
                        if method == 0:
                            name = request[1].decode()
                            assert name in d.METHODS and request[4] == b'dfmcp_dig_v1_16'
                            methods[len(methods) + 2] = name
                            response = {1: len(methods) + 1}
                        else:
                            name = methods[method]
                            self.calls.append(name)
                            assert request[1] == b't' * 32 and request[3] == 1 and request[4] == 16
                            response = {1: 1, 2: 0, 3: request[2], 4: 1, 5: 16, 6: self.source['generation'],
                                        7: self.source['df_version'].encode(), 8: self.source['dfhack_version'].encode()}
                            if name == 'ReadDesignation':
                                region = dict(zip(d.REGION_KEYS, (request[i] for i in range(5, 10))))
                                response[9] = self.capture(region)
                            elif name == 'PrepareDesignation':
                                key = request[11].decode()
                                if key in self.records:
                                    intent, effect = self.records[key]
                                    assert intent['plan_digest'] == request[13].hex()
                                    response.update({10: effect, 11: 1})
                                else:
                                    region = dict(zip(d.REGION_KEYS, (request[i] for i in range(5, 10))))
                                    intent = d.build_intent(self.address, key, region, bool(request[10]),
                                                            self.capture(region), self.source)
                                    assert intent['witness'] == request[12].hex()
                                    assert intent['plan_digest'] == request[13].hex()
                                    effect = native_record(intent, 0)
                                    self.records[key] = intent, effect
                                    response.update({10: effect, 11: 0})
                            elif name in ('QueryDesignation', 'CancelDesignation', 'CommitDesignation'):
                                key = request[11].decode()
                                if key in self.records:
                                    intent, effect = self.records[key]
                                    assert request[13].hex() == intent['plan_digest']
                                    if name != 'QueryDesignation':
                                        assert request[14].hex() == intent['prepare_token']
                                    state = d.effect(effect, intent)['state']
                                    if name == 'CommitDesignation' and state == 'prepared':
                                        assert self.capture(intent['region']) == bytes.fromhex(intent['observation_hex'])
                                        selected = b.cells(intent['region'])
                                        self.designated |= selected
                                        self.blocks |= {(x >> 4, y >> 4, z) for x, y, z in selected}
                                        self.sequence += 1
                                        effect = native_record(intent, 2)
                                    elif name == 'CancelDesignation' and state == 'prepared':
                                        effect = native_record(intent, 4, 2)
                                    self.records[key] = intent, effect
                                    if name == 'CommitDesignation' and self.lost_commit:
                                        self.lost_commit = False
                                        break
                                    response[10] = effect
                                    if name == 'CommitDesignation' and self.bad_effect:
                                        response[10] = effect[:-1] + bytes([effect[-1] ^ 1])
                        payload = d.encode(response)
                        data = struct.pack('<h2xi', -1, len(payload)) + payload
                        # Exercise the real client's fragmented header/body reads.
                        for start in range(0, len(data), 127):
                            sock.sendall(data[start:start + 127])
            except (ConnectionResetError, BrokenPipeError):
                pass  # Client deliberately fences malformed responses.
            except BaseException as cause:
                self.errors.append(repr(cause))


class BatchTests(unittest.TestCase):
    @contextmanager
    def setup(self, parts=None):
        with tempfile.TemporaryDirectory() as tmp, Peer() as peer:
            root = Path(tmp) / 'batch'; root.mkdir(mode=0o700)
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': 't' * 32,
                   'DFMCP_DIG_ENDPOINT': peer.address, 'DFMCP_DIG_ALLOW_DESIGNATE': '1'}
            with patch.dict(os.environ, env, clear=True):
                layout = b.Layout.from_json(specification(parts or [((5, 5, 2), (2, 2, 1)), ((9, 5, 3), (2, 2, 1))]))
                value = c.initialize(root, layout, 'region1', 2, False, c.POLICY, c.Budget(10000))
                yield root, peer, layout, value

    def review(self, root):
        return c.observe(root, c.Budget(10000))

    def advance(self, root, review):
        return c.advance(root, review['batch_id'], review['step'], review['observation']['witness'],
                         review['plan_digest_for_confirmation'], review['review_seal'], c.Budget(10000))

    def test_real_tcp_multi_level_lifecycle_reopens_after_each_step(self):
        with self.setup() as (root, peer, layout, initial):
            for index in range(2):
                review = self.review(root)
                self.assertEqual(review['step'], index)
                result = self.advance(root, review)
                self.assertEqual(result['effect_status'], 'designated')
                self.assertEqual(result['batch']['designated_steps'], index + 1)
            state = c.inspect(root, c.Budget(10000))
            self.assertEqual(state['phase'], 'designations_verified')
            self.assertEqual(peer.designated, set(layout.targets))
            self.assertEqual(peer.calls.count('CommitDesignation'), 2)
            self.assertFalse(state['excavation_completion_proven'])
            before = len(peer.calls)
            with patch.dict(os.environ, {}, clear=True):
                restored = c.recover(root, initial['batch_id'], 0, False, c.Budget(10000))
                self.assertEqual(restored['effect_status'], 'designated')
            self.assertEqual(before, len(peer.calls))

    def test_lost_reply_blocks_fresh_steps_then_recovers_without_recommit(self):
        with self.setup() as (root, peer, _, initial):
            review = self.review(root); peer.lost_commit = True
            with self.assertRaises((ValueError, OSError)):
                self.advance(root, review)
            state = c.inspect(root, c.Budget(10000))
            self.assertEqual(state['phase'], 'unresolved')
            before = len(peer.calls)
            with self.assertRaises(ValueError): self.review(root)
            with self.assertRaises(ValueError): self.advance(root, review)
            self.assertEqual(before, len(peer.calls))
            result = c.recover(root, initial['batch_id'], 0, False, c.Budget(10000))
            self.assertEqual(result['batch']['next_step'], 1)
            self.assertEqual(peer.calls.count('CommitDesignation'), 1)
            self.advance(root, self.review(root))
            self.assertEqual(peer.calls.count('CommitDesignation'), 2)

    def test_wrong_review_batch_or_step_refuses_before_native_connect(self):
        with self.setup() as (root, peer, _, _):
            review = self.review(root); before = len(peer.calls)
            for field, replacement in [('batch_id', '0' * 64), ('review_seal', '0' * 64),
                                       ('step', 1), ('plan_digest_for_confirmation', '0' * 64)]:
                with self.assertRaises(ValueError): self.advance(root, {**review, field: replacement})
            self.assertEqual(before, len(peer.calls))

    def test_changed_terrain_refuses_without_creating_an_intent(self):
        with self.setup() as (root, peer, _, _):
            review = self.review(root); peer.tick += 1
            with self.assertRaises(ValueError): self.advance(root, review)
            self.assertEqual(peer.calls.count('PrepareDesignation'), 0)
            self.assertEqual(c.inspect(root, c.Budget(10000))['records'], [])

    def test_whole_batch_map_preflight_before_creating_files(self):
        with tempfile.TemporaryDirectory() as tmp, Peer() as peer:
            root = Path(tmp)
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': 't' * 32,
                   'DFMCP_DIG_ENDPOINT': peer.address}
            with patch.dict(os.environ, env, clear=True):
                layout = b.Layout.from_json(specification([((5, 5, 2), (1, 1, 1)), ((5, 5, 15), (1, 1, 1))]))
                with self.assertRaises(ValueError):
                    c.initialize(root, layout, 'region1', 2, False, c.POLICY, c.Budget(10000))
                self.assertEqual(list(root.iterdir()), [])
                self.assertNotIn('PrepareDesignation', peer.calls)

    def test_fortress_incarnation_dimension_and_clock_changes_are_not_rebound(self):
        for field in ('site', 'folder', 'dimensions', 'sequence', 'tick', 'source'):
            with self.subTest(field=field), self.setup() as (root, peer, _, _):
                setattr(peer, field, {'site': 9, 'folder': 'other', 'dimensions': [65, 64, 16],
                    'sequence': 2, 'tick': 99, 'source': {**peer.source, 'generation': 42}}[field])
                with self.assertRaises(ValueError): self.review(root)
                self.assertNotIn('PrepareDesignation', peer.calls)

    def test_each_intent_registry_and_terminal_sync_failure(self):
        for failure in range(1, 7):
            with self.subTest(sync=failure), self.setup() as (root, peer, _, _):
                review = self.review(root)
                original, count = os.fsync, [0]
                def broken(fd):
                    count[0] += 1
                    if count[0] == failure:
                        raise OSError('injected sync failure')
                    return original(fd)
                with patch.object(os, 'fsync', broken), self.assertRaises(OSError):
                    self.advance(root, review)
                self.assertEqual(peer.calls.count('CommitDesignation'), 0 if failure <= 4 else 1)
                state = c.inspect(root, c.Budget(10000))
                self.assertEqual(state['phase'], 'unresolved' if failure <= 4 else 'ready')
                with self.assertRaises(ValueError): self.advance(root, review)

    def test_partial_intent_write_remains_a_fence_without_repair(self):
        with self.setup() as (root, peer, _, _):
            review = self.review(root)
            original, written = os.write, [False]
            def broken(fd, raw):
                if not written[0]:
                    written[0] = True
                    return original(fd, raw[:3])
                raise OSError('injected torn write')
            with patch.object(os, 'write', broken), self.assertRaises(OSError):
                self.advance(root, review)
            path = root / c.EFFECTS / 'step-000.json'
            raw = path.read_bytes(); self.assertEqual(len(raw), 3)
            with self.assertRaises(ValueError): c.inspect(root, c.Budget(10000))
            with self.assertRaises(ValueError): self.advance(root, review)
            self.assertEqual(path.read_bytes(), raw)
            self.assertNotIn('PrepareDesignation', peer.calls)

    def test_absent_native_record_is_unknown_and_cannot_advance(self):
        with self.setup() as (root, peer, _, initial):
            review = self.review(root)
            with patch.object(os, 'fsync', side_effect=OSError('sync')), self.assertRaises(OSError):
                self.advance(root, review)
            result = c.recover(root, initial['batch_id'], 0, False, c.Budget(10000))
            self.assertIsNone(result['effect'])
            self.assertEqual(result['batch']['phase'], 'unresolved')
            with self.assertRaises(ValueError): self.review(root)
            self.assertNotIn('CommitDesignation', peer.calls)

    def test_replayed_preparation_never_commits_and_cancel_aborts_batch(self):
        with self.setup() as (root, peer, _, initial):
            review = self.review(root)
            region = review['observation']['region']
            intent = d.build_intent(peer.address, review['key'], region, False, peer.capture(region), peer.source)
            peer.records[review['key']] = intent, native_record(intent, 0)
            result = self.advance(root, review)
            self.assertTrue(result['native_preparation_replayed'])
            self.assertNotIn('CommitDesignation', peer.calls)
            result = c.recover(root, initial['batch_id'], 0, True, c.Budget(10000))
            self.assertEqual(result['batch']['phase'], 'refused')
            with self.assertRaises(ValueError): self.review(root)

    def test_stop_is_durable_local_only_and_leaves_recovery_available(self):
        with self.setup() as (root, peer, _, initial):
            peer.lost_commit = True
            with self.assertRaises(ValueError): self.advance(root, self.review(root))
            before = len(peer.calls)
            with patch.dict(os.environ, {}, clear=True):
                result = c.stop(root, initial['batch_id'], c.Budget(10000))
                self.assertFalse(result['native_effects_cancelled'])
                self.assertTrue(c.inspect(root, c.Budget(10000))['stopped'])
                c.stop(root, initial['batch_id'], c.Budget(10000))
            self.assertEqual(before, len(peer.calls))
            result = c.recover(root, initial['batch_id'], 0, False, c.Budget(10000))
            self.assertIsNone(result['batch']['next_step'])
            self.assertTrue(result['batch']['stopped'])
            with self.assertRaises(ValueError): self.review(root)

    def test_environment_revocation_after_prepare_blocks_commit(self):
        with self.setup() as (root, peer, _, initial):
            review = self.review(root)
            original = d.Client.prepare
            def revoked(client, intent):
                result = original(client, intent)
                os.environ.pop('DFMCP_DIG_ALLOW_DESIGNATE')
                return result
            with patch.object(d.Client, 'prepare', revoked), self.assertRaises(ValueError):
                self.advance(root, review)
            self.assertEqual(peer.calls.count('PrepareDesignation'), 1)
            self.assertNotIn('CommitDesignation', peer.calls)
            self.assertEqual(c.inspect(root, c.Budget(10000))['phase'], 'unresolved')

    def test_manifest_tampering_after_prepare_cannot_reach_commit(self):
        with self.setup() as (root, peer, _, _):
            review = self.review(root); original = d.Client.prepare
            def changed(client, intent):
                result = original(client, intent)
                path = root / c.MANIFEST
                data = path.read_bytes(); path.write_bytes(data.replace(b'region1', b'region2'))
                # The bootstrap is hex, so force a same-length checksum change.
                if path.read_bytes() == data:
                    path.write_bytes(bytes([data[0] ^ 1]) + data[1:])
                return result
            with patch.object(d.Client, 'prepare', changed), self.assertRaises(ValueError):
                self.advance(root, review)
            self.assertNotIn('CommitDesignation', peer.calls)

    def test_corrupt_receipt_and_missing_registered_child_block_new_steps(self):
        for fault in ('receipt', 'missing', 'replacement'):
            with self.subTest(fault=fault), self.setup() as (root, peer, _, _):
                self.advance(root, self.review(root))
                if fault == 'receipt':
                    path = next((root / c.EFFECTS).glob('.dfmcp-dig-terminal-*'))
                    raw = path.read_bytes(); path.write_bytes(raw[:-2] + b'x\n')
                elif fault == 'missing':
                    (root / c.EFFECTS / 'step-000.json').rename(root.parent / 'removed.json')
                else:
                    (root / c.EFFECTS).rename(root.parent / 'old-effects')
                    (root / c.EFFECTS).mkdir(mode=0o700)
                before = len(peer.calls)
                with self.assertRaises((ValueError, OSError)): self.review(root)
                self.assertEqual(before, len(peer.calls))

    def test_lock_modes_symlinks_and_deadline_refuse_without_effects(self):
        with self.setup() as (root, peer, _, _):
            with s.open_store(d, root), self.assertRaises(BlockingIOError):
                c.inspect(root, c.Budget(10000))
            (root / c.MANIFEST).chmod(0o644)
            with self.assertRaises(ValueError): c.inspect(root, c.Budget(10000))
            (root / c.MANIFEST).chmod(0o600)
            link = root.parent / 'link'; link.symlink_to(root, target_is_directory=True)
            with self.assertRaises(OSError): c.inspect(link, c.Budget(10000))
            budget = c.Budget(1000); budget.deadline = 0
            with self.assertRaises(ValueError): c.observe(root, budget)
            self.assertNotIn('PrepareDesignation', peer.calls)

    def test_bad_native_receipt_stays_unknown_then_query_recovers(self):
        with self.setup() as (root, peer, _, initial):
            peer.bad_effect = True
            with self.assertRaises(ValueError): self.advance(root, self.review(root))
            self.assertEqual(c.inspect(root, c.Budget(10000))['phase'], 'unresolved')
            result = c.recover(root, initial['batch_id'], 0, False, c.Budget(10000))
            self.assertEqual(result['effect_status'], 'designated')
            self.assertEqual(peer.calls.count('CommitDesignation'), 1)

    def test_existing_single_designation_api_keeps_its_default_behavior(self):
        with self.setup() as (root, peer, _, _):
            independent = root.parent / 'single'; independent.mkdir(mode=0o700)
            region = dict(zip(d.REGION_KEYS, [20, 20, 2, 2, 2]))
            with d.Client(peer.address, b't' * 32, 10000) as client:
                observed = client.observe(region)
                witness = observed['observation']['witness']
                plan = d.plan_for(region, False, bytes.fromhex(witness)).hex()
                result = d.start(client, independent / 'one.json', 'one', region, False, witness, plan)
            self.assertEqual(result['effect_status'], 'designated')
            self.assertEqual(s.records_result(d, independent)['unresolved_records'], 0)

    def test_full_128_step_inventory_retains_every_receipt_within_output_bound(self):
        parts = [((2 + x * 2, 2, 2), (1, 1, 8)) for x in range(16)]
        with self.setup(parts) as (root, peer, layout, initial):
            self.assertEqual(len(layout.rectangles), 128)
            with c.open_batch(root, c.Budget(10000)) as batch:
                with s.open_store(d, batch.effects, True) as store:
                    for index, region in enumerate(layout.regions()):
                        intent = d.build_intent(peer.address, batch.step_key(index), region, False,
                                                peer.capture(region), peer.source)
                        with d.capsule(batch.effects / batch.step_name(index), intent) as owner:
                            store.register(owner)
                            d.terminal_receipt(owner, native_record(intent, 2))
                        selected = b.cells(region)
                        peer.designated |= selected
                        peer.blocks |= {(x >> 4, y >> 4, z) for x, y, z in selected}
                        peer.sequence += 1
            result = c.inspect(root, c.Budget(10000))
            self.assertEqual(result['phase'], 'designations_verified')
            self.assertEqual(len(result['records']), 128)
            METRICS['full_128_step_inventory_bytes'] = len(d.canonical(result))
            self.assertLessEqual(METRICS['full_128_step_inventory_bytes'], c.MAX_OUTPUT)
            self.assertNotIn('CommitDesignation', peer.calls)  # This test installs native byte fixtures.

    def test_hidden_or_known_hazard_targets_are_never_skipped(self):
        for field in ('hidden', 'hazards'):
            with self.subTest(field=field), self.setup() as (root, peer, _, _):
                getattr(peer, field).add((5, 5, 2))
                review = self.review(root)
                self.assertTrue(review['blockers'])
                with self.assertRaises(ValueError): self.advance(root, review)
                self.assertNotIn('PrepareDesignation', peer.calls)
                self.assertEqual(c.inspect(root, c.Budget(10000))['records'], [])

    def test_stop_marker_must_be_intact_and_recovery_bound(self):
        with self.setup() as (root, peer, _, initial):
            c.stop(root, initial['batch_id'], c.Budget(10000))
            value = {'format': 'dfmcp.dig-blueprint-stop/1', 'batch_id': '0' * 64}
            (root / c.STOP).write_bytes(c.file_bytes(value))
            with self.assertRaises(ValueError): self.review(root)
            self.assertNotIn('PrepareDesignation', peer.calls)

    def test_maximum_wire_fields_use_actual_bounded_json_serialization(self):
        with tempfile.TemporaryDirectory() as tmp, Peer() as peer:
            peer.maximum_fields = True
            peer.dimensions = [32768, 32768, 32768]
            peer.folder = '\x01' * 512
            peer.source['df_version'] = '\x01' * 128
            peer.source['dfhack_version'] = '\x02' * 128
            root = Path(tmp)
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': 't' * 32,
                   'DFMCP_DIG_ENDPOINT': peer.address}
            with patch.dict(os.environ, env, clear=True):
                layout = b.Layout.from_json(specification([((32758, 32758, 2), (8, 8, 1))]))
                c.initialize(root, layout, peer.folder, 2, False, c.POLICY, c.Budget(10000))
                review = self.review(root)
                METRICS['maximum_field_review_bytes'] = len(d.canonical(review))
                self.assertLessEqual(METRICS['maximum_field_review_bytes'], c.MAX_OUTPUT)
                self.assertEqual(len(review['observation']['cells']), 300)
                self.assertTrue(review['blockers'])
                self.assertNotIn('PrepareDesignation', peer.calls)

    def test_real_cli_initialization_requires_explicit_fortress_and_policy(self):
        with tempfile.TemporaryDirectory() as tmp, Peer() as peer:
            root = Path(tmp) / 'batch'; root.mkdir(mode=0o700)
            source = Path(tmp) / 'input.json'
            source.write_bytes(d.canonical(specification([((5, 5, 2), (2, 2, 1))])))
            env = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': 't' * 32,
                   'DFMCP_DIG_ENDPOINT': peer.address}
            command = [sys.executable, str(Path(c.__file__)), 'init', '--directory', str(root),
                       '--blueprint', str(source), '--world-folder', 'wrong', '--site', '2',
                       '--checkpoint-policy', c.POLICY]
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertFalse(json.loads(result.stdout)['ok'])
            self.assertEqual(list(root.iterdir()), [])
            command[command.index('wrong')] = 'region1'
            result = subprocess.run(command, env=env, capture_output=True, text=True, check=True, timeout=10)
            self.assertEqual(json.loads(result.stdout)['next_step'], 0)
            source.rename(Path(tmp) / 'input-no-longer-configured.json')
            result = subprocess.run([sys.executable, str(Path(c.__file__)), 'inspect', '--directory', str(root)],
                                    env={}, capture_output=True, text=True, check=True, timeout=10)
            self.assertEqual(json.loads(result.stdout)['target_tiles'], 4)
            self.assertNotIn('PrepareDesignation', peer.calls)

    def test_valid_later_receipt_cannot_hide_an_earlier_unresolved_intent(self):
        with self.setup() as (root, peer, _, _):
            with c.open_batch(root, c.Budget(10000)) as batch:
                with s.open_store(d, batch.effects, True) as store:
                    for index, region in enumerate(batch.layout.regions()):
                        intent = d.build_intent(peer.address, batch.step_key(index), region, False,
                                                peer.capture(region), peer.source)
                        with d.capsule(batch.effects / batch.step_name(index), intent) as owner:
                            store.register(owner)
                            if index == 1:
                                d.terminal_receipt(owner, native_record(intent, 2))
            before = len(peer.calls)
            with self.assertRaises(ValueError): c.inspect(root, c.Budget(10000))
            with self.assertRaises(ValueError): self.review(root)
            self.assertEqual(before, len(peer.calls))

    def test_real_cli_reopens_without_blueprint_input_file(self):
        with self.setup() as (root, peer, _, _):
            command = [sys.executable, str(Path(c.__file__)), 'observe', '--directory', str(root)]
            output = subprocess.run(command, capture_output=True, text=True, check=True, timeout=10)
            review = json.loads(output.stdout)
            command[2] = 'advance'
            command += ['--batch-id', review['batch_id'], '--step', str(review['step']),
                '--expected-witness', review['observation']['witness'], '--confirm-plan',
                review['plan_digest_for_confirmation'], '--review-seal', review['review_seal']]
            output = subprocess.run(command, capture_output=True, text=True, check=True, timeout=10)
            self.assertEqual(json.loads(output.stdout)['effect_status'], 'designated')
            self.assertEqual(peer.calls.count('CommitDesignation'), 1)
            with patch.dict(os.environ, {}, clear=True):
                output = subprocess.run([sys.executable, str(Path(c.__file__)), 'inspect', '--directory', str(root)],
                    capture_output=True, text=True, check=True, timeout=10)
            self.assertEqual(json.loads(output.stdout)['designated_steps'], 1)


if __name__ == '__main__':
    unittest.main()
