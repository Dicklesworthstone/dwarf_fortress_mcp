#!/usr/bin/env python3
"""Execute linked designation/terrain monitoring with both real client codecs.

One joined TCP peer implements both test protocols on one endpoint. Its world,
SDK and game are doubles, not a live DFHack compatibility qualification.
"""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import dig_blueprint as g
import dig_blueprint_client as c
import dig_designation_client as d
import dig_designation_store as s
import excavation_observer as e
import track_excavation as t
import track_dig_blueprint as m
from test_dig_blueprint import specification
from test_dig_blueprint_client import Peer, native_record

METRICS = {}


class CombinedPeer(Peer):
    def __init__(self):
        super().__init__()
        self.map_generation = 79  # Intentionally NOT the dig plugin's generation.
        self.floors, self.wet, self.active_dig, self.missing = set(), set(), set(), set()
        self.map_drop = False
        self.map_folder = self.map_site = self.map_dimensions = self.map_versions = None

    def map_capture(self, region):
        folder = self.folder if self.map_folder is None else self.map_folder
        site = self.site if self.map_site is None else self.map_site
        dimensions = self.dimensions if self.map_dimensions is None else self.map_dimensions
        raw = (b'DFMM1500' + struct.pack('>IIBI', self.tick // 403200, self.tick % 403200, 0, site)
               + d.text(folder.encode()) + struct.pack('>III', *dimensions)
               + struct.pack('>IIIIIII', *region.origin, *region.size, region.volume))
        x, y, z = region.origin
        w, h, levels = region.size
        for pz in range(z, z + levels):
            for py in range(y, y + h):
                for px in range(x, x + w):
                    point = (px, py, pz)
                    if point in self.missing:
                        raw += b'\0'
                    elif point in self.hidden:
                        raw += b'\1'
                    else:
                        floor = point in self.floors
                        dig = point in self.active_dig or (point in self.designated and not floor)
                        raw += b'\2' + struct.pack('>IBBBBBBBIHH', 42, 3 if floor else 2,
                            int(point in self.wet), 0, 0, int(dig), 0, 0, 1, 10015, 10015)
        return raw

    def dig_reply(self, name, request, response):
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
                intent = d.build_intent(self.address, key, region, bool(request[10]), self.capture(region), self.source)
                assert intent['witness'] == request[12].hex() and intent['plan_digest'] == request[13].hex()
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
                    selected = g.cells(intent['region'])
                    self.designated |= selected
                    self.blocks |= {(x >> 4, y >> 4, z) for x, y, z in selected}
                    self.sequence += 1
                    effect = native_record(intent, 2)
                elif name == 'CancelDesignation' and state == 'prepared':
                    effect = native_record(intent, 4, 2)
                self.records[key] = intent, effect
                if name == 'CommitDesignation' and self.lost_commit:
                    self.lost_commit = False
                    return False
                response[10] = effect
        return True

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
                    methods, selected_plugin = {}, None
                    while not self.done.is_set():
                        header = self.read(sock, 8)
                        if header is None:
                            break
                        method, length = struct.unpack('<h2xi', header)
                        assert 0 <= length <= 2048
                        request = d.decode(self.read(sock, length), 14)
                        if method == 0:
                            plugin = request[4]
                            assert plugin in (b'dfmcp_dig_v1_16', b'dfmcp_map_v1_5')
                            assert selected_plugin is None or selected_plugin == plugin
                            selected_plugin = plugin
                            name = request[1].decode()
                            package = b'dfmcp.map.v1_5' if plugin == b'dfmcp_map_v1_5' else b'dfmcp.dig.v1_16'
                            assert request[2] == package + b'.Request' and request[3] == package + b'.Reply'
                            assert name in (('Handshake', 'ReadObservation') if plugin == b'dfmcp_map_v1_5' else d.METHODS)
                            methods[len(methods) + 2] = name
                            response = {1: len(methods) + 1}
                        else:
                            name = methods[method]
                            is_map = selected_plugin == b'dfmcp_map_v1_5'
                            self.calls.append(('Map.' if is_map else '') + name)
                            assert request[1] == (b'm' * 32 if is_map else b't' * 32)
                            assert request[3] == 1 and request[4] == (5 if is_map else 16)
                            versions = self.map_versions if is_map and self.map_versions is not None else self.source
                            response = {1: 1, 2: 0, 3: request[2], 4: 1, 5: request[4],
                                6: self.map_generation if is_map else self.source['generation'],
                                7: versions['df_version'].encode(), 8: versions['dfhack_version'].encode()}
                            if is_map:
                                if name == 'ReadObservation':
                                    if self.map_drop:
                                        self.map_drop = False
                                        break
                                    region = e.Region(tuple(request[i] for i in range(5, 8)), tuple(request[i] for i in range(8, 11)))
                                    response[9] = self.map_capture(region)
                            elif not self.dig_reply(name, request, response):
                                break
                        payload = d.encode(response)
                        data = struct.pack('<h2xi', -1, len(payload)) + payload
                        for start in range(0, len(data), 127):
                            sock.sendall(data[start:start + 127])
            except (ConnectionResetError, BrokenPipeError):
                pass
            except BaseException as cause:
                self.errors.append(repr(cause))


def environment(peer, mapping=True):
    if mapping:
        return {'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': '1', 'DFMCP_MAP_TOKEN': 'm' * 32,
                'DFMCP_MAP_ENDPOINT': peer.address}
    return {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16': '1', 'DFMCP_DIG_TOKEN': 't' * 32,
            'DFMCP_DIG_ENDPOINT': peer.address, 'DFMCP_DIG_ALLOW_DESIGNATE': '1'}


def designate_one(root):
    review = c.observe(root, c.Budget(10000))
    return c.advance(root, review['batch_id'], review['step'], review['observation']['witness'],
        review['plan_digest_for_confirmation'], review['review_seal'], c.Budget(10000))


class LinkedTests(unittest.TestCase):
    @contextmanager
    def setup(self, *, complete=True, parts=None, fixtures=False):
        with tempfile.TemporaryDirectory() as tmp, CombinedPeer() as peer:
            root, monitor = Path(tmp) / 'batch', Path(tmp) / 'monitor'
            root.mkdir(mode=0o700); monitor.mkdir(mode=0o700)
            layout = g.Layout.from_json(specification(parts or [((5, 5, 2), (2, 2, 1)), ((9, 5, 3), (2, 2, 1))]))
            with patch.dict(os.environ, environment(peer, False), clear=True):
                initial = c.initialize(root, layout, 'region1', 2, False, c.POLICY, c.Budget(10000))
                if complete:
                    if fixtures:
                        with c.open_batch(root, c.Budget(30000)) as batch:
                            with s.open_store(d, batch.effects, True) as store:
                                for index, region in enumerate(layout.regions()):
                                    intent = d.build_intent(peer.address, batch.step_key(index), region, False, peer.capture(region), peer.source)
                                    with d.capsule(batch.effects / batch.step_name(index), intent) as owner:
                                        store.register(owner); d.terminal_receipt(owner, native_record(intent, 2))
                                    peer.sequence += 1
                                    peer.designated |= g.cells(region)
                                    peer.blocks |= {(x >> 4, y >> 4, z) for x, y, z in g.cells(region)}
                    else:
                        for _ in layout.rectangles:
                            designate_one(root)
            with patch.dict(os.environ, environment(peer), clear=True):
                yield root, monitor, peer, initial['batch_id'], layout

    def start(self, root, monitor, batch_id, **options):
        return m.start(monitor, root, batch_id, **{'max_game_ticks': 20, 'stable_ticks': 2,
            'required_samples': 2, 'max_gap_ticks': 10, **options})

    def test_actual_multi_level_designation_to_completion_with_restarts(self):
        with self.setup() as (root, monitor, peer, batch_id, layout):
            before = peer.calls.count('CommitDesignation')
            initial = self.start(root, monitor, batch_id)
            self.assertEqual(initial['goal_status'], 'pending')
            self.assertTrue(initial['historical_designations_verified'])
            self.assertEqual(initial['association']['dig_generation'], 41)
            self.assertEqual(initial['association']['map_generation'], 79)
            self.assertFalse(initial['association']['shared_process_identity_proven'])
            peer.floors = set(layout.targets); peer.tick += 1
            self.assertEqual(m.sample(monitor)['goal_status'], 'stabilizing')
            peer.tick += 2
            done = m.sample(monitor)
            self.assertTrue(done['designation_and_sampled_goal_evidence_verified'])
            self.assertEqual(done['goal_status'], 'satisfied')
            self.assertFalse(done['mining_action_completed_proven'])
            self.assertFalse(done['association']['mining_causality_proven'])
            self.assertEqual(before, peer.calls.count('CommitDesignation'))
            self.assertEqual(peer.calls.count('Map.ReadObservation'), 3)
            before = len(peer.calls)
            with patch.dict(os.environ, {}, clear=True):
                self.assertEqual(m.sample(monitor)['goal_status'], 'satisfied')
                self.assertEqual(m.inspect(monitor)['journal_head'], done['journal_head'])
            self.assertEqual(before, len(peer.calls))
            # Original, unchanged standalone tracker can decode this same journal.
            self.assertEqual(t.inspect(monitor / m.JOURNAL)['journal_head'], done['journal_head'])

    def test_incomplete_unknown_and_refused_batches_cannot_start_a_monitor(self):
        for phase in ('ready', 'unknown', 'refused'):
            with self.subTest(phase=phase), self.setup(complete=False) as (root, monitor, peer, batch_id, _):
                with patch.dict(os.environ, environment(peer, False), clear=True):
                    if phase == 'unknown':
                        peer.lost_commit = True
                        with self.assertRaises(ValueError): designate_one(root)
                    elif phase == 'refused':
                        review = c.observe(root, c.Budget(10000))
                        region = review['observation']['region']
                        intent = d.build_intent(peer.address, review['key'], region, False, peer.capture(region), peer.source)
                        peer.records[review['key']] = intent, native_record(intent, 4, 2)
                        designate_one(root)
                before = len(peer.calls)
                with self.assertRaises(ValueError): self.start(root, monitor, batch_id)
                self.assertEqual(before, len(peer.calls))
                self.assertEqual(list(monitor.iterdir()), [])

    def test_wrong_batch_identity_and_impossible_limits_refuse_before_native_io(self):
        options = [{'batch_id': '0' * 64}, {'stable_ticks': 21}, {'required_samples': 22},
                   {'required_samples': True}, {'max_gap_ticks': 0}, {'max_game_ticks': 0},
                   {'max_game_ticks': 403200, 'stable_ticks': 129, 'max_gap_ticks': 1}]
        with self.setup() as (root, monitor, peer, batch_id, _):
            before = len(peer.calls)
            for values in options:
                with self.subTest(values=values), self.assertRaises(ValueError):
                    self.start(root, monitor, values.get('batch_id', batch_id),
                               **{k: v for k, v in values.items() if k != 'batch_id'})
            self.assertEqual(before, len(peer.calls))
            self.assertEqual(list(monitor.iterdir()), [])

    def test_initial_scope_software_and_clock_must_match_without_equating_generations(self):
        for name, value in [('map_site', 3), ('map_folder', 'other'), ('map_dimensions', [65, 64, 16]),
                            ('map_versions', {'df_version': 'other', 'dfhack_version': 'dfhack-test'}), ('tick', 99)]:
            with self.subTest(name=name), self.setup() as (root, monitor, peer, batch_id, _):
                setattr(peer, name, value)
                with self.assertRaises(ValueError): self.start(root, monitor, batch_id)
                self.assertEqual(list(monitor.iterdir()), [])
                self.assertEqual(peer.calls.count('Map.ReadObservation'), 1)

    def test_explicit_endpoint_and_profile_isolation_remain_closed(self):
        with self.setup() as (root, monitor, peer, batch_id, _):
            before = len(peer.calls)
            for update in [{'DFMCP_MAP_ENDPOINT': '127.0.0.1:1'}, {'DFMCP_DIG_ALLOW_DESIGNATE': '1'},
                           {'DFMCP_ADMITTED_BRIDGE_PROTOCOL': '1.0'}]:
                with patch.dict(os.environ, update), self.assertRaises(ValueError):
                    self.start(root, monitor, batch_id)
            self.assertEqual(before, len(peer.calls))
            self.assertEqual(list(monitor.iterdir()), [])

    def test_simultaneous_sparse_mask_not_stitched_success_and_holes_ignored(self):
        with self.setup() as (root, monitor, peer, batch_id, layout):
            self.start(root, monitor, batch_id)
            parts = [{p for p in layout.targets if p[2] == z} for z in (2, 3)]
            for selected in parts:
                peer.tick += 1; peer.floors = selected
                self.assertEqual(m.sample(monitor)['goal_status'], 'pending')
            peer.floors = set(layout.targets)
            peer.hidden.add((7, 5, 2))  # Inside bounding box, outside sparse targets.
            peer.tick += 1
            self.assertEqual(m.sample(monitor)['goal_status'], 'stabilizing')
            peer.tick += 2
            result = m.sample(monitor)
            self.assertTrue(result['blueprint_goal_satisfied_at_sample'])
            self.assertEqual(result['counts_at_last_observation']['hidden'], 0)
            self.assertGreater(result['blueprint_at_last_observation']['unselected_tiles'], 0)

    def test_wet_or_designated_floors_and_unknown_targets_reset_stability(self):
        for field in ('wet', 'active_dig', 'hidden', 'missing'):
            with self.subTest(field=field), self.setup() as (root, monitor, peer, batch_id, layout):
                peer.floors = set(layout.targets)
                self.start(root, monitor, batch_id)
                peer.tick += 1
                getattr(peer, field).add(layout.targets[0])
                result = m.sample(monitor)
                self.assertEqual(result['matching_samples'], 0)
                self.assertEqual(result['goal_status'], 'unknown' if field in ('hidden', 'missing') else 'pending')
                getattr(peer, field).clear(); peer.tick += 1
                self.assertEqual(m.sample(monitor)['goal_status'], 'stabilizing')
                peer.tick += 2
                self.assertEqual(m.sample(monitor)['goal_status'], 'satisfied')

    def test_same_tick_samples_do_not_inflate_and_gap_restarts_stability(self):
        with self.setup() as (root, monitor, peer, batch_id, layout):
            peer.floors = set(layout.targets)
            initial = self.start(root, monitor, batch_id, stable_ticks=0, required_samples=3, max_gap_ticks=2)
            self.assertEqual(initial['matching_samples'], 1)
            self.assertEqual(m.sample(monitor)['matching_samples'], 1)
            peer.tick += 1
            self.assertEqual(m.sample(monitor)['matching_samples'], 2)
            peer.tick += 3
            result = m.sample(monitor)
            self.assertEqual(result['matching_samples'], 1)
            self.assertEqual(result['interruption'], 'sample_gap_reset')

    def test_fixed_deadline_and_source_or_clock_invalidation_are_persistent(self):
        for change in ('deadline', 'generation', 'clock', 'folder'):
            with self.subTest(change=change), self.setup() as (root, monitor, peer, batch_id, layout):
                initial = self.start(root, monitor, batch_id)
                if change == 'deadline': peer.tick = initial['goal']['deadline_tick'] + 1
                if change == 'generation': peer.map_generation += 1
                if change == 'clock': peer.tick -= 1
                if change == 'folder': peer.map_folder = 'another'
                peer.floors = set(layout.targets)
                result = m.sample(monitor)
                self.assertEqual(result['goal_status'], 'expired' if change == 'deadline' else 'invalidated')
                self.assertEqual(result['goal']['deadline_tick'], initial['goal']['deadline_tick'])
                before = len(peer.calls)
                with patch.dict(os.environ, {}, clear=True):
                    self.assertEqual(m.sample(monitor)['journal_head'], result['journal_head'])
                self.assertEqual(before, len(peer.calls))
                self.assertFalse(result['designation_and_sampled_goal_evidence_verified'])

    def test_lost_read_resets_streak_and_never_retries_or_mutates(self):
        with self.setup() as (root, monitor, peer, batch_id, layout):
            peer.floors = set(layout.targets)
            self.start(root, monitor, batch_id)
            peer.map_drop = True; peer.tick += 1
            result = m.sample(monitor)
            self.assertFalse(result['ok'])
            self.assertEqual(result['goal_status'], 'unknown')
            self.assertEqual(result['matching_samples'], 0)
            self.assertEqual(result['interruption'], 'read_failed')
            self.assertFalse(result['pending_read'])
            self.assertEqual(peer.calls.count('Map.ReadObservation'), 2)
            peer.tick += 1
            self.assertEqual(m.sample(monitor)['matching_samples'], 1)
            self.assertEqual(peer.calls.count('CommitDesignation'), len(layout.rectangles))

    def test_interrupted_process_retains_read_intent_before_socket_work(self):
        with self.setup() as (root, monitor, peer, batch_id, layout):
            peer.floors = set(layout.targets)
            self.start(root, monitor, batch_id)
            def crash(*args):
                history = t.replay((monitor / m.JOURNAL).read_bytes())
                self.assertTrue(history.pending_read)
                raise KeyboardInterrupt('simulated process interruption before connect')
            with self.assertRaises(KeyboardInterrupt): m.sample(monitor, connect=crash)
            result = m.inspect(monitor)
            self.assertEqual(result['matching_samples'], 0)
            self.assertTrue(result['pending_read'])
            peer.tick += 2
            self.assertEqual(m.sample(monitor)['matching_samples'], 1)
            self.assertEqual(peer.calls.count('Map.ReadObservation'), 2)

    def test_each_initial_publication_sync_failure_preserves_original_goal(self):
        for fault in range(1, 5):
            with self.subTest(fault=fault), self.setup() as (root, monitor, peer, batch_id, _):
                original, calls = os.fsync, [0]
                def broken(fd):
                    path = Path(os.readlink(f'/proc/self/fd/{fd}'))
                    if path == monitor or path.parent == monitor:
                        calls[0] += 1
                        if calls[0] == fault: raise OSError('injected monitor sync')
                    return original(fd)
                with patch.object(os, 'fsync', broken), self.assertRaises(OSError):
                    self.start(root, monitor, batch_id)
                raw = (monitor / m.JOURNAL).read_bytes()
                history = t.replay(raw)
                self.assertEqual(history.goal.deadline_tick, 120)
                with self.assertRaises(ValueError): self.start(root, monitor, batch_id, max_game_ticks=40)
                self.assertEqual((monitor / m.JOURNAL).read_bytes(), raw)
                if fault <= 2:
                    with self.assertRaises(ValueError): m.inspect(monitor)
                else:
                    self.assertEqual(m.inspect(monitor)['journal_id'], history.identity)
                self.assertEqual(peer.calls.count('Map.ReadObservation'), 1)

    def test_read_sample_sync_failure_never_acknowledges_unverified_progress(self):
        # sync(existing journal+link), read_started, and sample each have file/dir syncs.
        for fault in range(1, 9):
            with self.subTest(fault=fault), self.setup() as (root, monitor, peer, batch_id, layout):
                self.start(root, monitor, batch_id)
                peer.floors = set(layout.targets); peer.tick += 1
                original, calls = os.fsync, [0]
                def broken(fd):
                    path = Path(os.readlink(f'/proc/self/fd/{fd}'))
                    if path == monitor or path.parent == monitor:
                        calls[0] += 1
                        if calls[0] == fault: raise OSError('injected progress sync')
                    return original(fd)
                with patch.object(os, 'fsync', broken), self.assertRaises(OSError): m.sample(monitor)
                self.assertEqual(peer.calls.count('Map.ReadObservation'), 1 if fault <= 6 else 2)
                # Reopening recomputes whatever complete bytes actually survived; no deadline reset.
                result = m.inspect(monitor)
                self.assertEqual(result['goal']['deadline_tick'], 120)
                self.assertFalse(result['blueprint_goal_satisfied_at_sample'])

    def test_torn_progress_write_is_never_repaired_or_overwritten(self):
        with self.setup() as (root, monitor, peer, batch_id, _):
            self.start(root, monitor, batch_id)
            original, wrote = os.write, [False]
            def broken(fd, raw):
                if Path(os.readlink(f'/proc/self/fd/{fd}')) == monitor / m.JOURNAL:
                    if wrote[0]: raise OSError('torn progress write')
                    wrote[0] = True
                    return original(fd, raw[:5])
                return original(fd, raw)
            with patch.object(os, 'write', broken), self.assertRaises(OSError): m.sample(monitor)
            raw = (monitor / m.JOURNAL).read_bytes()
            with self.assertRaises(ValueError): m.inspect(monitor)
            with self.assertRaises(ValueError): m.sample(monitor)
            with self.assertRaises(ValueError): self.start(root, monitor, batch_id)
            self.assertEqual(raw, (monitor / m.JOURNAL).read_bytes())
            self.assertEqual(peer.calls.count('Map.ReadObservation'), 1)

    def test_batch_receipt_corruption_during_map_read_prevents_success(self):
        with self.setup() as (root, monitor, peer, batch_id, layout):
            self.start(root, monitor, batch_id)
            peer.floors = set(layout.targets); peer.tick += 1
            original = e.MapClient.observe
            def changed(client):
                result = original(client)
                path = next((root / c.EFFECTS).glob('.dfmcp-dig-terminal-*'))
                raw = path.read_bytes(); path.write_bytes(raw[:-2] + b'x\n')
                return result
            with patch.object(e.MapClient, 'observe', changed), self.assertRaises(ValueError): m.sample(monitor)
            self.assertTrue(t.inspect(monitor / m.JOURNAL)['pending_read'])
            with self.assertRaises(ValueError): m.inspect(monitor)
            before = len(peer.calls)
            with patch.dict(os.environ, {}, clear=True):
                result = m.cancel(monitor)
            self.assertEqual(result['goal_status'], 'cancelled')
            self.assertFalse(result['historical_designations_verified'])
            self.assertEqual(before, len(peer.calls))

    def test_journal_copy_link_tampering_and_batch_directory_replacement_fail_closed(self):
        for fault in ('journal_copy', 'link', 'batch_replacement'):
            with self.subTest(fault=fault), self.setup() as (root, monitor, peer, batch_id, _):
                self.start(root, monitor, batch_id)
                if fault == 'journal_copy':
                    path = monitor / m.JOURNAL; raw = path.read_bytes()
                    path.rename(monitor.parent / 'original-progress')
                    path.write_bytes(raw); path.chmod(0o600)
                elif fault == 'link':
                    path = monitor / m.LINK
                    value = json.loads(path.read_bytes())['value']; value['batch_id'] = '0' * 64
                    path.write_bytes(c.file_bytes(value))
                else:
                    root.rename(root.parent / 'original-batch'); root.mkdir(mode=0o700)
                before = len(peer.calls)
                with self.assertRaises((ValueError, OSError)): m.sample(monitor)
                self.assertEqual(before, len(peer.calls))

    def test_cancel_remains_offline_when_original_batch_is_unavailable(self):
        with self.setup() as (root, monitor, peer, batch_id, _):
            self.start(root, monitor, batch_id)
            root.rename(root.parent / 'removed-batch')
            before = len(peer.calls)
            with patch.dict(os.environ, {}, clear=True):
                with self.assertRaises(OSError): m.inspect(monitor)
                cancelled = m.cancel(monitor)
                self.assertEqual(cancelled['goal_status'], 'cancelled')
                self.assertFalse(cancelled['designation_evidence']['verified_this_call'])
                self.assertEqual(m.cancel(monitor)['journal_head'], cancelled['journal_head'])
            self.assertEqual(before, len(peer.calls))

    def test_local_batch_stop_does_not_invalidate_immutable_designation_link(self):
        with self.setup() as (root, monitor, peer, batch_id, _):
            initial = self.start(root, monitor, batch_id)
            with patch.dict(os.environ, {}, clear=True):
                c.stop(root, batch_id, c.Budget(10000))
                result = m.inspect(monitor)
            self.assertEqual(result['designation_evidence'], initial['designation_evidence'])
            self.assertEqual(result['journal_id'], initial['journal_id'])

    def test_locked_files_modes_and_extra_entries_are_rejected(self):
        with self.setup() as (root, monitor, peer, batch_id, _):
            self.start(root, monitor, batch_id)
            before = len(peer.calls)
            with s.open_store(d, monitor), self.assertRaises(BlockingIOError): m.sample(monitor)
            with t.open_journal(monitor / m.JOURNAL, t.Budget(10000)), self.assertRaises(BlockingIOError):
                m.sample(monitor)
            (monitor / m.LINK).chmod(0o644)
            with self.assertRaises(ValueError): m.sample(monitor)
            (monitor / m.LINK).chmod(0o600)
            (monitor / 'extra').write_text('must not be ignored')
            with self.assertRaises(ValueError): m.inspect(monitor)
            self.assertEqual(before, len(peer.calls))

    def test_complete_128_step_receipt_inventory_projects_within_output_bound(self):
        parts = [((2 + x * 2, 2, 2), (1, 1, 8)) for x in range(16)]
        with self.setup(parts=parts, fixtures=True) as (root, monitor, peer, batch_id, layout):
            result = self.start(root, monitor, batch_id, timeout_ms=30000)
            self.assertEqual(result['designation_evidence']['total_steps'], 128)
            self.assertEqual(result['blueprint_digest'], layout.blueprint_digest)
            METRICS['complete_128_step_linked_output_bytes'] = len(d.canonical(result))
            self.assertLessEqual(METRICS['complete_128_step_linked_output_bytes'], m.MAX_OUTPUT)
            self.assertEqual(peer.calls.count('CommitDesignation'), 0)  # Installed fixtures, not native execution.
            self.assertEqual(peer.calls.count('Map.ReadObservation'), 1)

    def test_full_512_targets_1024_cell_capture_and_journal_replay(self):
        parts = [((2, 2, 2), (16, 16, 1)), ((2, 2, 5), (16, 16, 1))]
        with self.setup(parts=parts) as (root, monitor, peer, batch_id, layout):
            first = self.start(root, monitor, batch_id)
            self.assertEqual(first['blueprint_at_last_observation']['captured_tiles'], 1024)
            self.assertEqual(first['blueprint_at_last_observation']['target_tiles'], 512)
            peer.floors = set(layout.targets); peer.tick += 1
            m.sample(monitor); peer.tick += 2
            result = m.sample(monitor)
            self.assertTrue(result['designation_and_sampled_goal_evidence_verified'])
            self.assertEqual(result['counts_at_last_observation']['matched'], 512)
            METRICS['full_volume_linked_output_bytes'] = len(d.canonical(result))
            METRICS['full_volume_three_sample_journal_bytes'] = (monitor / m.JOURNAL).stat().st_size
            self.assertLessEqual(METRICS['full_volume_linked_output_bytes'], m.MAX_OUTPUT)
            self.assertEqual(t.inspect(monitor / m.JOURNAL)['goal_status'], 'satisfied')

    def test_real_cli_start_sample_offline_inspect_and_cancel(self):
        with self.setup() as (root, monitor, peer, batch_id, _):
            base = [sys.executable, str(Path(m.__file__))]
            result = subprocess.run(base + ['start', '--directory', str(monitor), '--batch', str(root),
                '--batch-id', batch_id, '--max-game-ticks', '20'], capture_output=True, text=True, timeout=15, check=True)
            initial = json.loads(result.stdout)
            result = subprocess.run(base + ['sample', '--directory', str(monitor)], capture_output=True,
                                    text=True, timeout=15, check=True)
            sampled = json.loads(result.stdout)
            self.assertEqual(initial['journal_id'], sampled['journal_id'])
            for operation in ('inspect', 'cancel'):
                result = subprocess.run(base + [operation, '--directory', str(monitor)], env={},
                                        capture_output=True, text=True, timeout=15, check=True)
                self.assertTrue(json.loads(result.stdout)['ok'])
            self.assertEqual(json.loads(result.stdout)['goal_status'], 'cancelled')
            self.assertFalse(json.loads(result.stdout)['historical_designations_verified'])
            self.assertEqual(peer.calls.count('Map.ReadObservation'), 2)


if __name__ == '__main__':
    unittest.main()
