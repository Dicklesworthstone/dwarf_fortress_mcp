"""Real existing CLI/TCP client over a joined synthetic map/1.5 peer.

This is protocol/process component evidence, not a DFHack plugin or live fort.
"""
from __future__ import annotations

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
import unittest
from unittest.mock import patch

from test_room_access_layout import MANIFEST, SCRIPTS, payload, request, tile
import excavation_observer as native
import room_layout_observed as observed
import survey_room_layout as survey
from furniture_plan import canonical
from room_provisioning import RoomPlan

TOKEN = 'map-fixture-token-' + 'x'*32


def read_exact(stream, count):
    out = bytearray()
    while len(out) < count:
        part = stream.recv(count-len(out))
        if not part: raise AssertionError('fixture request truncated')
        out.extend(part)
    return bytes(out)


class Peer:
    def __init__(self, value, raw, fault=None):
        self.value, self.raw, self.fault = value, raw, fault
        self.calls, self.errors = [], []
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.bind(('127.0.0.1', 0)); self.listener.listen(1); self.listener.settimeout(5)
        self.address = '127.0.0.1:' + str(self.listener.getsockname()[1])
        self.thread = threading.Thread(target=self.run)

    def __enter__(self):
        self.thread.start(); return self

    def __exit__(self, *_):
        self.thread.join(7)
        self.listener.close()
        if self.thread.is_alive():
            self.thread.join(6)
            raise AssertionError('map fixture did not quiesce')
        if self.errors: raise self.errors[0]

    def run(self):
        try:
            with self.listener.accept()[0] as stream:
                stream.settimeout(5)
                assert read_exact(stream, 12) == b'DFHack?\n' + struct.pack('<i', 1)
                stream.sendall(b'DFHack!\n' + struct.pack('<i', 1))
                for expected in ('Handshake', 'ReadObservation'):
                    method, fields = self.receive(stream)
                    assert method == 0
                    assert fields == {1:expected.encode(), 2:b'dfmcp.map.v1_5.Request',
                                      3:b'dfmcp.map.v1_5.Reply', 4:b'dfmcp_map_v1_5'}
                    self.calls.append('bind:' + expected)
                    self.reply(stream, {1:2 if expected == 'Handshake' else 3})
                nonce = None
                for method_id, name in ((2,'Handshake'), (3,'ReadObservation')):
                    method, fields = self.receive(stream)
                    assert method == method_id and fields[1] == TOKEN.encode()
                    if nonce is None: nonce = fields[2]
                    assert fields[2] == nonce and len(nonce) == 32 and (fields[3],fields[4]) == (1,5)
                    region = self.value['region']
                    assert [fields[i] for i in range(5,11)] == region['origin'] + region['size']
                    assert fields[11] == max(1024, 575+20*region['size'][0]*region['size'][1])
                    self.calls.append(name)
                    reply = {1:1, 2:0, 3:nonce, 4:1, 5:5, 6:MANIFEST.generation,
                             7:MANIFEST.df_version.encode(), 8:MANIFEST.dfhack_version.encode()}
                    if name == 'ReadObservation':
                        reply[9] = self.raw
                        if self.fault == 'nonce': reply[3] = b'bad'
                        elif self.fault == 'generation': reply[6] += 1
                        elif self.fault == 'protocol': reply[5] = 4
                        elif self.fault == 'drop': return
                    self.reply(stream, reply)
                # A second read, mutation or retry on this connection is a test
                # failure. Real CLI closure is required before the peer exits.
                assert stream.recv(1) == b''
                self.calls.append('closed')
        except BaseException as error:
            self.errors.append(error)

    @staticmethod
    def receive(stream):
        method, size = struct.unpack('<h2xi', read_exact(stream, 8))
        assert 0 <= size <= 2048
        return method, native.decode(read_exact(stream, size), 11)

    @staticmethod
    def reply(stream, value):
        raw = native.encode(value)
        stream.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)


def environment(peer):
    return {**{k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')},
            survey.OPT_IN:'1', survey.ENDPOINT:peer.address, survey.TOKEN:TOKEN,
            'PYTHONDONTWRITEBYTECODE':'1'}


def command(path, peer=None, *, emit='survey', saved=False):
    env = ({k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')}
           if peer is None else environment(peer))
    return subprocess.run([sys.executable, str(SCRIPTS/'survey_room_layout.py'),
        '--survey-file' if saved else '--request-file', str(path), '--emit', emit],
        env=env, capture_output=True, timeout=12)


class AccessTcpTests(unittest.TestCase):
    def test_one_actual_read_then_offline_replay_and_every_exact_export(self):
        value = request(); raw = payload(value)
        expected = observed.solve(canonical(value), raw, MANIFEST)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'request.json'; source.write_bytes(canonical(value))
            with Peer(value, raw) as peer: run = command(source, peer)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(run.stdout, expected.encode())
            self.assertEqual(peer.calls, ['bind:Handshake','bind:ReadObservation','Handshake','ReadObservation','closed'])
            saved = Path(directory)/'survey.json'; saved.write_bytes(run.stdout)
            for kind in survey.EXPORTS:
                exported = command(saved, emit=kind, saved=True)
                self.assertEqual(exported.returncode, 0, exported.stderr)
                self.assertEqual(exported.stdout, survey.export(expected, kind, False))
                self.assertEqual(exported.stderr, b'')
                if kind == 'room-plan':
                    plan = RoomPlan.decode(exported.stdout)
                    self.assertEqual(plan.json()['intent']['access']['point'], value['access_point'])
                if kind == 'report':
                    reply = json.loads(exported.stdout)
                    self.assertTrue(reply['result']['access_evidence']['point_floor_observed'])
                    self.assertFalse(reply['agent_turn']['coverage']['native_acquisition_verified_this_call'])

    def test_native_report_binds_entrance_without_claiming_current_reachability(self):
        value = request()
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'request.json'; source.write_bytes(canonical(value))
            with Peer(value, payload(value)) as peer: run = command(source, peer, emit='report')
            self.assertEqual(run.returncode, 0)
            reply = json.loads(run.stdout)
            self.assertTrue(reply['agent_turn']['coverage']['native_acquisition_verified_this_call'])
            self.assertTrue(reply['result']['access_evidence']['same_capture_as_layout'])
            self.assertFalse(reply['result']['access_evidence']['current_access_proven'])
            self.assertNotIn(TOKEN.encode(), run.stdout + run.stderr)

    def test_ten_unusable_entrance_states_refuse_over_actual_tcp(self):
        states = [tile(presence=0),tile(presence=1),tile(shape=2),tile(shape=1),tile(shape=8),
                  tile(shape=3,depth=1),tile(shape=3,magma=1),tile(shape=3,dig=1),
                  tile(shape=3,building=1),tile(shape=3,units=1)]
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'request.json'
            for row in states:
                value = request()
                # An operator floor permission cannot certify a native wall.
                value['site_mask'] = {'region':value['region'], 'cells':'F'+'.'*59}
                source.write_bytes(canonical(value))
                with Peer(value, payload(value,{(10,10,2):row})) as peer: run = command(source, peer)
                with self.subTest(tile=row.hex()):
                    self.assertEqual(run.returncode, 2)
                    reply = json.loads(run.stdout)
                    self.assertIsNone(reply['result']); self.assertFalse(reply['ok'])
                    self.assertEqual(peer.calls.count('ReadObservation'), 1)
                    self.assertNotIn(TOKEN.encode(), run.stdout+run.stderr)
                    self.assertNotIn(str(source).encode(), run.stdout+run.stderr)

    def test_lost_response_and_source_confusion_do_not_return_cached_layout(self):
        value = request()
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'request.json'; source.write_bytes(canonical(value))
            for fault in ('nonce','generation','protocol','drop'):
                with Peer(value, payload(value), fault) as peer: run = command(source, peer)
                with self.subTest(fault=fault):
                    self.assertEqual(run.returncode, 2)
                    self.assertIsNone(json.loads(run.stdout)['result'])
                    self.assertEqual(peer.calls.count('ReadObservation'), 1)

    def test_real_connection_closes_before_layout_search(self):
        value = request(); closed = []
        close, solve = survey._MapClient.close, observed.solve
        def tracked_close(client):
            close(client); closed.append(client.closed)
        def check_solve(*args, **kwargs):
            self.assertTrue(closed and all(closed))
            return solve(*args, **kwargs)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'request.json'; source.write_bytes(canonical(value))
            with Peer(value, payload(value)) as peer, patch.dict(os.environ, environment(peer), clear=True):
                stream = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')
                with patch.object(survey._MapClient,'close',tracked_close), patch.object(observed,'solve',check_solve):
                    with patch.object(sys,'stdout',stream): code = survey.main(['--request-file',str(source)])
                self.assertEqual(code, 0)
                stream.flush(); self.assertTrue(json.loads(stream.buffer.getvalue())['ok'])

    def test_authority_revocation_after_capture_prevents_publication(self):
        value = request(); close = survey._MapClient.close
        def revoke_after_close(client):
            close(client); os.environ[survey.TOKEN] = 'revoked-'+'z'*32
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'request.json'; source.write_bytes(canonical(value))
            with Peer(value,payload(value)) as peer, patch.dict(os.environ,environment(peer),clear=True):
                stream = io.TextIOWrapper(io.BytesIO(),encoding='utf-8')
                with patch.object(survey._MapClient,'close',revoke_after_close), patch.object(sys,'stdout',stream):
                    code = survey.main(['--request-file',str(source)])
                self.assertEqual(code, 2)
                stream.flush(); self.assertIsNone(json.loads(stream.buffer.getvalue())['result'])
                self.assertEqual(peer.calls.count('ReadObservation'), 1)


if __name__ == '__main__': unittest.main()
