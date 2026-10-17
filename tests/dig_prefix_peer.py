"""Joined synthetic dig/1.16 peer for real client/private-file tests, not DFHack."""
from __future__ import annotations

import hashlib
import socket
import struct
import threading

import dig_designation_client as d

TOKEN = 'synthetic-dig-prefix-credential-0001'
SOURCE = {'generation': 41, 'df_version': 'fixture-df', 'dfhack_version': 'fixture-dfhack'}


def blueprint(count=3):
    return {'schema': 'dfmcp.excavation-blueprint/1', 'parts': [
        {'region': {'origin': [4 + i * 3, 4, 2], 'size': [2, 2, 1]}, 'shape': 'floor'}
        for i in range(count)]}


def effect_bytes(intent, state, reason=0, after=None):
    original = bytes.fromhex(intent['observation_hex'])
    before = d.observation(original, intent['region'])
    prefix = (b'DFMDGE16' + struct.pack('>QQQ', before['generation'], before['sequence'], before['tick'])
              + d.region_bytes(intent['region']) + bytes([intent['allow_hidden_neighbors']])
              + bytes.fromhex(intent['witness'] + intent['plan_digest'] + intent['prepare_token']))
    count = intent['region']['width'] * intent['region']['height'] if state == 2 else 0
    witness = hashlib.sha256(after).digest() if state == 2 else bytes(32)
    body = bytes([state, reason, int(state == 2)]) + struct.pack('>I', count) + witness
    receipt = (d.digest(b'dfmcp-dig-designation-receipt/1', struct.pack('>Q', before['generation'])
               + d.key_bytes(intent['key']) + bytes.fromhex(intent['plan_digest'] + intent['prepare_token']) + body)
               if state in (2, 4) else bytes(32))
    return prefix + body + receipt + d.key_bytes(intent['key'])


class DigPeer:
    """Every connection and worker is joined; effects persist across connections."""
    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.bind(('127.0.0.1', 0))
        self.sock.listen(8)
        self.sock.settimeout(0.1)
        self.address = f'127.0.0.1:{self.sock.getsockname()[1]}'
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._serve)
        self.sequence, self.tick = 0, 100
        self.designated, self.scheduled, self.records = set(), set(), {}
        self.calls, self.commits, self.connection_commits, self.errors = [], [], [], []
        self.mode, self.at_step = None, 0
        self.block_region = None
        self.on_operation = None
        self.after_commit = None
        self.closed_connections = 0
        self.thread.start()

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.stop.set()
        self.sock.close()
        self.thread.join(5)
        if self.thread.is_alive():
            raise AssertionError('synthetic native peer did not join')
        if self.errors:
            raise AssertionError(self.errors)

    @staticmethod
    def receive(conn, size):
        out = bytearray()
        while len(out) < size:
            raw = conn.recv(size - len(out))
            if not raw:
                return None
            out += raw
        return bytes(out)

    def capture(self, selected, *, changed=False):
        x, y, z, width, height = d.region(selected)
        folder = b'region1'
        output = (b'DFMDG016' + struct.pack('>QQQI3I', SOURCE['generation'], self.sequence,
                  self.tick + int(changed), 2, 256, 256, 16) + d.region_bytes(selected)
                  + b'\1' + struct.pack('>H', len(folder)) + folder
                  + struct.pack('>H', (width + 2) * (height + 2) * 3))
        cells = []
        for pz in range(z - 1, z + 2):
            for py in range(y - 1, y + height + 1):
                for px in range(x - 1, x + width + 1):
                    targeted = (px, py, pz) in self.designated
                    scheduled = (px >> 4, py >> 4, pz) in self.scheduled
                    occupied = self.block_region == selected and (px, py, pz) == (x, y, z)
                    cells.append(b'\2' + struct.pack('>IIIIIIHHBBB', 42, 0, 0,
                        4000 if targeted else 0, 0 if scheduled else 7, 0, 10015, 10015,
                        int(targeted), 0, 1 | (16 if scheduled else 0) | (4 if occupied else 0)))
        return output + b''.join(cells)

    def _serve(self):
        while not self.stop.is_set():
            try:
                conn, _ = self.sock.accept()
            except socket.timeout:
                continue
            except OSError:
                if not self.stop.is_set():
                    self.errors.append('unexpected listener failure')
                return
            try:
                with conn:
                    conn.settimeout(5)
                    self._connection(conn)
            except (BrokenPipeError, ConnectionResetError):
                # The real owner may reject authority/evidence before reading.
                pass
            except BaseException as error:
                self.errors.append(repr(error))
            finally:
                self.closed_connections += 1

    def _connection(self, conn):
        greeting = self.receive(conn, 12)
        if greeting is None:
            return
        assert greeting == b'DFHack?\n' + struct.pack('<i', 1)
        conn.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        methods = {index + 2: name for index, name in enumerate(d.METHODS)}
        last_capture, reads, commits = None, 0, 0
        self.connection_commits.append(0)
        connection_index = len(self.connection_commits) - 1
        while not self.stop.is_set():
            header = self.receive(conn, 8)
            if header is None:
                return
            method, size = struct.unpack('<h2xi', header)
            assert 0 <= size <= 2048
            raw = self.receive(conn, size)
            if raw is None:
                return
            fields = d.decode(raw, 14)
            if method == 0:
                name = fields[1].decode('ascii')
                assert name in d.METHODS and fields[4] == b'dfmcp_dig_v1_16'
                result = {1: d.METHODS.index(name) + 2}
            else:
                operation = methods[method]
                assert fields[1] == TOKEN.encode() and fields[3] == 1 and fields[4] == 16
                self.calls.append((connection_index, operation))
                if self.on_operation is not None:
                    self.on_operation(operation, self)
                step = len(self.commits)
                active = step == self.at_step
                if self.mode == 'drop_read' and active and operation == 'ReadDesignation':
                    return
                generation = SOURCE['generation'] + int(self.mode == 'generation' and active)
                result = {1: 1, 2: 0, 3: fields[2], 4: 1, 5: 16, 6: generation,
                          7: SOURCE['df_version'].encode(), 8: SOURCE['dfhack_version'].encode()}
                if operation == 'ReadDesignation':
                    selected = dict(zip(d.REGION_KEYS, [fields[key] for key in range(5, 10)]))
                    reads += 1
                    last_capture = self.capture(selected, changed=self.mode == 'change_read' and active and reads == 2)
                    result[9] = last_capture
                elif operation == 'PrepareDesignation':
                    selected = dict(zip(d.REGION_KEYS, [fields[key] for key in range(5, 10)]))
                    assert last_capture is not None
                    key = fields[11].decode('ascii')
                    intent = d.build_intent(self.address, key, selected, bool(fields[10]), last_capture, SOURCE)
                    assert fields[12] == bytes.fromhex(intent['witness']) and fields[13] == bytes.fromhex(intent['plan_digest'])
                    receipt = effect_bytes(intent, 0)
                    self.records[key] = intent, receipt
                    result.update({10: receipt, 11: int(self.mode == 'prepared_replay' and active)})
                    if self.mode == 'drop_prepare' and active:
                        return
                elif operation == 'CommitDesignation':
                    key = fields[11].decode('ascii')
                    intent, _ = self.records[key]
                    assert fields[14] == bytes.fromhex(intent['prepare_token'])
                    commits += 1
                    self.connection_commits[connection_index] = commits
                    assert commits == 1, 'client repeated a commit on one native connection'
                    assert key not in self.commits, 'duplicate original native key'
                    self.commits.append(key)
                    if self.mode == 'refuse' and active:
                        receipt = effect_bytes(intent, 4, 1)
                    elif self.mode == 'unknown' and active:
                        receipt = effect_bytes(intent, 1)
                    else:
                        x, y, z, width, height = d.region(intent['region'])
                        self.designated.update((px, py, z) for py in range(y, y + height) for px in range(x, x + width))
                        self.scheduled.update((bx, by, z) for by in range(y >> 4, ((y + height - 1) >> 4) + 1)
                                              for bx in range(x >> 4, ((x + width - 1) >> 4) + 1))
                        self.sequence += 1
                        receipt = effect_bytes(intent, 2, after=self.capture(intent['region']))
                    self.records[key] = intent, receipt
                    if self.after_commit is not None:
                        self.after_commit(key, self)
                    if self.mode == 'drop_commit' and active:
                        return
                    if self.mode == 'bad_receipt' and active:
                        receipt = receipt[:175] + bytes([receipt[175] ^ 1]) + receipt[176:]
                    result[10] = receipt
                elif operation in ('QueryDesignation', 'CancelDesignation'):
                    key = fields[11].decode('ascii')
                    if key in self.records:
                        intent, receipt = self.records[key]
                        assert fields[13] == bytes.fromhex(intent['plan_digest'])
                        if operation == 'CancelDesignation' and d.effect(receipt, intent)['state'] == 'prepared':
                            receipt = effect_bytes(intent, 4, 2)
                            self.records[key] = intent, receipt
                        result[10] = receipt
                else:
                    assert operation == 'Handshake'
            payload = d.encode(result)
            conn.sendall(struct.pack('<h2xi', -1, len(payload)) + payload)
