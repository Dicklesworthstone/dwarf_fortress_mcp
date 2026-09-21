#!/usr/bin/env python3
"""One-shot, opt-in dig/1.16 developer client; never an MCP or production runner.

Only a NEW private intent capsule can start a designation. Recovery commands
query or retire the existing preparation, never replay CommitDesignation.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import socket
import stat
import struct
import sys
import time
from typing import Iterator

METHODS = ('Handshake', 'ReadDesignation', 'PrepareDesignation', 'CommitDesignation',
           'QueryDesignation', 'CancelDesignation')
REGION_KEYS = ('x', 'y', 'z', 'width', 'height')
ENVIRONMENT = {'DFMCP_ALLOW_UNADMITTED_DIG_V1_16', 'DFMCP_DIG_TOKEN',
               'DFMCP_DIG_ENDPOINT', 'DFMCP_DIG_ALLOW_DESIGNATE'}
MAX_TICK = (2**32 - 1) * 403200 + 403199
MAX_CAPSULE = 65536
MAX_OUTPUT = 131072
INTENT_KEYS = {'format', 'endpoint', 'key', 'region', 'allow_hidden_neighbors',
               'observation_hex', 'witness', 'plan_digest', 'prepare_token',
               'manifest', 'effect_status'}


class Rejected(ValueError):
    """A local/protocol/custody refusal, not a negative game-effect receipt."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Rejected(message)


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'integer outside dig profile bounds')
    return value  # type: ignore[return-value]


def flag(value: object) -> bool:
    require(type(value) is bool, 'boolean must be explicit true or false')
    return value  # type: ignore[return-value]


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def digest(domain: bytes, data: bytes) -> bytes:
    return hashlib.sha256(domain + b'\0' + data).digest()


def exact_hex(value: object, minimum: int, maximum: int | None = None) -> bytes:
    maximum = minimum if maximum is None else maximum
    require(isinstance(value, str) and minimum * 2 <= len(value) <= maximum * 2
            and len(value) % 2 == 0 and re.fullmatch('[0-9a-f]+', value) is not None,
            'noncanonical hexadecimal field')
    return bytes.fromhex(value)


def utf8(raw: object, maximum: int) -> str:
    require(isinstance(raw, bytes) and 1 <= len(raw) <= maximum and b'\0' not in raw,
            'invalid bounded text field')
    try:
        return raw.decode('utf-8')
    except UnicodeDecodeError as cause:
        raise Rejected('invalid UTF-8 field') from cause


def text(raw: bytes) -> bytes:
    require(len(raw) <= 65535, 'text length cannot be encoded')
    return struct.pack('>H', len(raw)) + raw


def key_bytes(value: object) -> bytes:
    require(isinstance(value, str) and re.fullmatch('[A-Za-z0-9_.-]{1,128}', value) is not None,
            'invalid designation key')
    return text(value.encode('ascii'))


def region(value: object) -> tuple[int, int, int, int, int]:
    require(isinstance(value, dict) and set(value) == set(REGION_KEYS), 'invalid region fields')
    x, y, z, width, height = (integer(value[k], 1, 32767) for k in REGION_KEYS)
    require(width <= 8 and height <= 8 and x <= 32767 - width
            and y <= 32767 - height and z <= 32766, 'region or full halo exceeds tile bounds')
    return x, y, z, width, height


def region_bytes(value: dict) -> bytes:
    return struct.pack('>IIIII', *region(value))


class Reader:
    def __init__(self, raw: bytes):
        self.raw, self.offset = raw, 0

    def take(self, size: int) -> bytes:
        require(0 <= size <= len(self.raw) - self.offset, 'truncated native record')
        out = self.raw[self.offset:self.offset + size]
        self.offset += size
        return out

    def scalar(self, fmt: str) -> int:
        return struct.unpack('>' + fmt, self.take(struct.calcsize('>' + fmt)))[0]

    def string(self, maximum: int) -> str:
        return utf8(self.take(self.scalar('H')), maximum)

    def finish(self) -> None:
        require(self.offset == len(self.raw), 'trailing native record bytes')


def observation(raw: bytes, selected: dict) -> dict:
    """Decode complete target + halo evidence. Hidden cells have NO payload."""
    x, y, z, width, height = region(selected)
    require(isinstance(raw, bytes) and len(raw) <= 16384, 'observation exceeds 16 KiB')
    r = Reader(raw)
    require(r.take(8) == b'DFMDG016', 'not a dig/1.16 observation')
    generation = integer(r.scalar('Q'), 1, 2**64 - 2)
    sequence = integer(r.scalar('Q'), 0, 2**64 - 2)
    tick = integer(r.scalar('Q'), 0, MAX_TICK)
    site = integer(r.scalar('I'), 0, 2**31 - 1)
    dimensions = [r.scalar('I') for _ in range(3)]
    require(r.take(20) == region_bytes(selected), 'native observation substituted the requested region')
    require(x + width < dimensions[0] <= 32768 and y + height < dimensions[1] <= 32768
            and z + 1 < dimensions[2] <= 32768, 'halo outside observed map dimensions')
    paused = bool(integer(r.scalar('B'), 0, 1))
    folder = r.string(512)
    require(r.scalar('H') == (width + 2) * (height + 2) * 3, 'incomplete native halo')
    cells = []
    for pz in range(z - 1, z + 2):
        for py in range(y - 1, y + height + 1):
            for px in range(x - 1, x + width + 1):
                offset = r.offset
                presence = integer(r.scalar('B'), 0, 2)
                cell = {'coordinate': [px, py, pz], 'presence': presence, 'offset': offset}
                if presence == 2:
                    values = struct.unpack('>IIIIIIHHBBB', r.take(31))
                    for k, v in zip(('tiletype', 'designation_other', 'occupancy', 'priority', 'cooldown',
                                     'block_other', 'temperature1', 'temperature2', 'dig', 'hazards', 'flags'), values):
                        cell[k] = v
                    require(cell['priority'] <= 7000 and cell['dig'] <= 7
                            and cell['hazards'] <= 15 and cell['flags'] <= 31, 'invalid visible tile fields')
                cells.append(cell)
    r.finish()
    return {'generation': generation, 'sequence': sequence, 'tick': tick, 'site': site,
            'dimensions': dimensions, 'paused': paused, 'folder': folder, 'region': dict(selected),
            'cells': cells, 'witness': hashlib.sha256(raw).hexdigest()}


def blockers(o: dict, allow_hidden: bool) -> list[str]:
    flag(allow_hidden)
    x, y, z, width, height = region(o['region'])
    result = set() if o['paused'] else {'unpaused'}
    for cell in o['cells']:
        px, py, pz = cell['coordinate']
        target = pz == z and x <= px < x + width and y <= py < y + height
        if cell['presence'] == 0:
            result.add('missing_context')
        if cell['presence'] == 1 and not allow_hidden:
            result.add('unacknowledged_hidden_context')
        if cell['presence'] == 2 and cell['hazards']:
            result.add('known_visible_hazard')
        if target:
            if cell['presence'] != 2:
                result.add('unobserved_target')
            else:
                if not cell['flags'] & 1:
                    result.add('not_natural_wall')
                if cell['dig'] or cell['flags'] & 2:
                    result.add('existing_designation')
                if cell['flags'] & 12:
                    result.add('occupied_or_job')
    if o['sequence'] >= 2**64 - 2:
        result.add('intervention_sequence_exhausted')
    return sorted(result)


def expected_after(raw: bytes, selected: dict) -> bytes:
    o = observation(raw, selected)
    require(o['sequence'] < 2**64 - 2, 'designation sequence cannot advance')
    x, y, z, width, height = region(selected)
    out = bytearray(raw)
    struct.pack_into('>Q', out, 16, o['sequence'] + 1)
    for cell in o['cells']:
        if cell['presence'] != 2:
            continue
        px, py, pz = cell['coordinate']
        offset = cell['offset']
        if pz == z and x <= px < x + width and y <= py < y + height:
            struct.pack_into('>I', out, offset + 13, 4000)
            out[offset + 29] = 1
        if pz == z and x >> 4 <= px >> 4 <= (x + width - 1) >> 4 and y >> 4 <= py >> 4 <= (y + height - 1) >> 4:
            struct.pack_into('>I', out, offset + 17, 0)
            out[offset + 31] |= 16
    return bytes(out)


def plan_for(selected: dict, allow_hidden: bool, witness: bytes) -> bytes:
    require(len(witness) == 32, 'invalid witness width')
    return digest(b'dfmcp-dig-designation-plan/1', region_bytes(selected) + bytes([flag(allow_hidden)]) + witness)


def token_for(generation: int, key: str, plan: bytes) -> bytes:
    integer(generation, 1, 2**64 - 2)
    require(len(plan) == 32, 'invalid plan width')
    return digest(b'dfmcp-dig-designation-token/1', struct.pack('>Q', generation) + key_bytes(key) + plan)[:16]


def endpoint(value: object) -> tuple[str, int]:
    require(isinstance(value, str) and len(value) <= 128, 'invalid endpoint')
    match = re.fullmatch(r'([0-9.]+):([0-9]{1,5})', value)
    require(match is not None, 'endpoint must be numeric IPv4 loopback with a nonzero port')
    try:
        address = ipaddress.IPv4Address(match[1])
    except ipaddress.AddressValueError as cause:
        raise Rejected('invalid numeric loopback address') from cause
    port = integer(int(match[2]), 1, 65535)
    require(address.is_loopback and f'{address}:{port}' == value, 'non-loopback or noncanonical endpoint')
    return str(address), port


def manifest(value: object) -> dict:
    require(isinstance(value, dict) and set(value) == {'generation', 'df_version', 'dfhack_version'},
            'invalid source manifest')
    integer(value['generation'], 1, 2**64 - 2)
    for field in ('df_version', 'dfhack_version'):
        require(isinstance(value[field], str), 'invalid source version type')
        utf8(value[field].encode('utf-8'), 128)
    return value


def verify_intent(value: object) -> dict:
    require(isinstance(value, dict) and set(value) == INTENT_KEYS, 'invalid dig intent fields')
    require(value['format'] == 'dfmcp.dig-intent/1'
            and value['effect_status'] == 'indeterminate_until_native_query', 'intent cannot claim a terminal effect')
    endpoint(value['endpoint']); key_bytes(value['key']); flag(value['allow_hidden_neighbors'])
    raw = exact_hex(value['observation_hex'], 1, 16384)
    o = observation(raw, value['region'])
    require(not blockers(o, value['allow_hidden_neighbors']), 'ineligible designation intent')
    require(manifest(value['manifest'])['generation'] == o['generation'], 'intent source mismatch')
    witness = exact_hex(value['witness'], 32)
    require(witness == hashlib.sha256(raw).digest(), 'intent observation checksum mismatch')
    plan = plan_for(value['region'], value['allow_hidden_neighbors'], witness)
    require(exact_hex(value['plan_digest'], 32) == plan
            and exact_hex(value['prepare_token'], 16) == token_for(o['generation'], value['key'], plan),
            'intent plan or token commitment mismatch')
    return value


def build_intent(address: str, key: str, selected: dict, allow_hidden: bool, raw: bytes, source: dict) -> dict:
    o = observation(raw, selected)
    plan = plan_for(selected, allow_hidden, bytes.fromhex(o['witness']))
    return verify_intent({'format': 'dfmcp.dig-intent/1', 'endpoint': address, 'key': key,
        'region': dict(selected), 'allow_hidden_neighbors': allow_hidden, 'observation_hex': raw.hex(),
        'manifest': dict(source), 'witness': o['witness'], 'plan_digest': plan.hex(),
        'prepare_token': token_for(o['generation'], key, plan).hex(),
        'effect_status': 'indeterminate_until_native_query'})


def effect(raw: bytes, intent: dict) -> dict:
    """A self-consistent hash is insufficient: verify the exact allowed post-state."""
    verify_intent(intent)
    require(isinstance(raw, bytes) and 207 <= len(raw) <= 334, 'invalid effect extent')
    o = observation(bytes.fromhex(intent['observation_hex']), intent['region'])
    prefix = (b'DFMDGE16' + struct.pack('>QQQ', o['generation'], o['sequence'], o['tick'])
              + region_bytes(intent['region']) + bytes([intent['allow_hidden_neighbors']])
              + bytes.fromhex(intent['witness'] + intent['plan_digest'] + intent['prepare_token']))
    require(raw[:133] == prefix, 'native effect differs from complete retained intent')
    r = Reader(raw); r.take(133)
    state, reason, known = (r.scalar('B') for _ in range(3))
    count = r.scalar('I'); after = r.take(32); receipt = r.take(32); key = r.string(128); r.finish()
    require(key == intent['key'] and state in (0, 1, 2, 4), 'effect key or state mismatch')
    require((state == 4 and reason in (1, 2) or state != 4 and reason == 0)
            and known == int(state == 2), 'effect reason/readback presence contradicts state')
    if state == 2:
        expected = hashlib.sha256(expected_after(bytes.fromhex(intent['observation_hex']), intent['region'])).digest()
        require(count == intent['region']['width'] * intent['region']['height'] and after == expected,
                'designation does not have exact terrain, priority and block-scheduling readback')
    else:
        require(count == 0 and after == bytes(32), 'absent effect readback has nonzero backing fields')
    proof = digest(b'dfmcp-dig-designation-receipt/1', struct.pack('>Q', o['generation']) + key_bytes(key)
                   + bytes.fromhex(intent['plan_digest'] + intent['prepare_token']) + raw[133:172])
    require(receipt == (proof if state in (2, 4) else bytes(32)), 'invalid effect receipt checksum')
    return {'state': {0: 'prepared', 1: 'unknown', 2: 'designated', 4: 'refused'}[state],
            'reason': {0: 'none', 1: 'stale', 2: 'cancelled_before_dispatch'}[reason],
            'key': key, 'plan_digest': intent['plan_digest'], 'observation_witness': intent['witness'],
            'designated_count': count if known else None, 'after_witness': after.hex() if known else None,
            'receipt_digest': receipt.hex() if state in (2, 4) else None,
            'historical_designation_readback': bool(known), 'current_terrain_unproved': True,
            'excavation_completion_proven': False, 'retry_commit_permitted': False}


def varint(value: int) -> bytes:
    integer(value, 0, 2**64 - 1)
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128); value >>= 7
    out.append(value)
    return bytes(out)


def encode(fields: dict[int, int | bytes]) -> bytes:
    out = bytearray()
    for key, value in sorted(fields.items()):
        integer(key, 1, 14)
        if isinstance(value, bytes):
            out += varint(key << 3 | 2) + varint(len(value)) + value
        else:
            out += varint(key << 3) + varint(value)
    return bytes(out)


def decode(raw: bytes, maximum: int = 11) -> dict:
    r = Reader(raw)
    def number() -> int:
        start, value = r.offset, 0
        for shift in range(0, 70, 7):
            byte = r.scalar('B')
            require(shift < 63 or byte <= 1, 'protobuf integer overflow')
            value |= (byte & 127) << shift
            if byte < 128:
                require(r.raw[start:r.offset] == varint(value), 'nonminimal protobuf integer')
                return value
        raise Rejected('unterminated protobuf integer')
    fields = {}
    while r.offset < len(raw):
        tag = number(); key, wire = tag >> 3, tag & 7
        require(1 <= key <= maximum and key not in fields, 'unknown or duplicate protobuf field')
        require(wire in (0, 2), 'unsupported protobuf wire type')
        fields[key] = number() if wire == 0 else r.take(number())
    return fields


class Client:
    """Bounded numeric-loopback connection; every wire/evidence error fences it."""
    def __init__(self, address: str, token: bytes, timeout_ms: int):
        selected = endpoint(address)
        integer(timeout_ms, 1, 60000)
        require(isinstance(token, bytes) and 32 <= len(token) <= 256, 'invalid credential bound')
        self.address, self.token, self.nonce = address, token, secrets.token_bytes(32)
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.methods: dict[str, int] = {}
        self.manifest: dict | None = None
        self.closed, self.commit_attempted = False, False
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.sock.settimeout(self.remaining()); self.sock.connect(selected)
            self.send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self.read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'invalid DFHack greeting')
            for name in METHODS:
                binding = decode(self.frame(0, encode({1: name.encode('ascii'), 2: b'dfmcp.dig.v1_16.Request',
                                  3: b'dfmcp.dig.v1_16.Reply', 4: b'dfmcp_dig_v1_16'})), 1)
                require(set(binding) == {1}, 'invalid native method binding')
                method = integer(binding[1], 2, 32767)
                require(method not in self.methods.values(), 'native method IDs alias')
                self.methods[name] = method
            self._invoke('Handshake', {})
        except BaseException:
            self.close(); raise

    def close(self) -> None:
        if not self.closed:
            self.closed = True; self.sock.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_args) -> None:
        self.close()

    def remaining(self) -> float:
        require(not self.closed, 'dig connection fenced; recovery requires an explicit new connection')
        left = self.deadline - time.monotonic()
        require(left > 0, 'shared dig connection deadline exhausted')
        return left

    def send(self, data: bytes) -> None:
        self.sock.settimeout(self.remaining()); self.sock.sendall(data)

    def read(self, size: int) -> bytes:
        out = bytearray()
        while len(out) < size:
            self.sock.settimeout(self.remaining()); part = self.sock.recv(size - len(out))
            require(bool(part), 'native reply ended before its complete frame')
            out += part
        return bytes(out)

    def frame(self, method: int, request: bytes) -> bytes:
        require(len(request) <= 2048, 'native request exceeds 2 KiB')
        self.send(struct.pack('<h2xi', method, len(request)) + request)
        count, total = 0, 0
        while True:
            kind, size = struct.unpack('<h2xi', self.read(8))
            require(kind in (-1, -3), 'DFHack refused RPC or emitted an unknown frame')
            require(0 <= size <= (32768 if kind == -1 else 65536), 'native frame bound exceeded')
            if kind == -3:
                count += 1; total += size
                require(count <= 8 and total <= 262144, 'native notification allowance exhausted')
            payload = self.read(size)
            if kind == -1:
                self.remaining(); return payload

    def _invoke(self, operation: str, fields: dict, intent: dict | None = None) -> dict:
        require(operation in METHODS, 'method outside fixed designation profile')
        self.remaining()
        require(all(type(k) is int and 5 <= k <= 14 for k in fields), 'cannot override profile credentials')
        try:
            response = decode(self.frame(self.methods[operation], encode({1: self.token, 2: self.nonce, 3: 1, 4: 16, **fields})))
            require(set(range(1, 9)) <= set(response), 'required reply fields missing')
            require(response[3] == self.nonce and response[4] == 1 and response[5] == 16, 'reply nonce/profile mismatch')
            accepted = integer(response[1], 0, 1); code = integer(response[2], 0, 8)
            if not accepted:
                require(code != 0 and set(response) == set(range(1, 9)) and response[6] == 0
                        and response[7] == b'' and response[8] == b'', 'noncanonical native refusal')
                raise Rejected(f'native refusal code {code}; no effect outcome inferred')
            require(code == 0, 'successful reply carries an error')
            current = manifest({'generation': integer(response[6], 1, 2**64 - 2),
                'df_version': utf8(response[7], 128), 'dfhack_version': utf8(response[8], 128)})
            require(self.manifest is None or current == self.manifest, 'native incarnation/software changed; do not redispatch')
            extras = {'Handshake': set(), 'ReadDesignation': {9}, 'PrepareDesignation': {10, 11},
                      'CommitDesignation': {10}, 'CancelDesignation': {10}, 'QueryDesignation': {10} if 10 in response else set()}[operation]
            require(set(response) == set(range(1, 9)) | extras, 'unexpected or missing native reply payload')
            result = {'manifest': current}
            if 9 in response:
                raw = response[9]
                require(isinstance(raw, bytes), 'invalid observation protobuf type')
                selected = dict(zip(REGION_KEYS, (fields[k] for k in range(5, 10))))
                observed = observation(raw, selected)
                require(observed['generation'] == current['generation'], 'observation generation mismatch')
                result.update(observation=observed, raw=raw)
            if 10 in response:
                require(intent is not None and current == intent['manifest'], 'receipt source differs from intent')
                result.update(effect=effect(response[10], intent), effect_raw=response[10])
            if 11 in response:
                result['replayed'] = bool(integer(response[11], 0, 1))
                require(result['replayed'] or result['effect']['state'] == 'prepared', 'fresh preparation has a historical outcome')
            if operation == 'CommitDesignation':
                require(result['effect']['state'] != 'prepared', 'commit acknowledgement is not an effect outcome')
            self.remaining(); self.manifest = current
            return result
        except BaseException:
            self.close(); raise

    def observe(self, selected: dict) -> dict:
        region(selected)
        return self._invoke('ReadDesignation', dict(zip(range(5, 10), region(selected))))

    def check_intent(self, intent: dict) -> None:
        verify_intent(intent); self.remaining()
        require(intent['manifest'] == self.manifest and intent['endpoint'] == self.address,
                'intent source/endpoint differs from current connection; historical outcome remains unknown')

    def prepare(self, intent: dict) -> dict:
        self.check_intent(intent)
        fields = dict(zip(range(5, 10), region(intent['region'])))
        fields.update({10: int(intent['allow_hidden_neighbors']), 11: intent['key'].encode(),
                       12: bytes.fromhex(intent['witness']), 13: bytes.fromhex(intent['plan_digest'])})
        return self._invoke('PrepareDesignation', fields, intent)

    def commit(self, intent: dict, preparation: dict) -> dict:
        self.check_intent(intent)
        require(not self.commit_attempted and preparation.get('replayed') is False
                and effect(preparation.get('effect_raw'), intent)['state'] == 'prepared',
                'only a fresh exact preparation can enter one-shot dispatch')
        self.commit_attempted = True
        return self._invoke('CommitDesignation', self.effect_fields(intent, True), intent)

    @staticmethod
    def effect_fields(intent: dict, token: bool) -> dict:
        fields = {11: intent['key'].encode(), 13: bytes.fromhex(intent['plan_digest'])}
        if token:
            fields[14] = bytes.fromhex(intent['prepare_token'])
        return fields

    def query(self, intent: dict) -> dict:
        self.check_intent(intent)
        return self._invoke('QueryDesignation', self.effect_fields(intent, False), intent)

    def cancel(self, intent: dict) -> dict:
        self.check_intent(intent)
        return self._invoke('CancelDesignation', self.effect_fields(intent, True), intent)


class Capsule:
    def __init__(self, path: Path, fd: int, parent: int, raw: bytes, intent: dict):
        self.path, self.fd, self.parent, self.raw, self.intent = path, fd, parent, raw, intent
        self.identity = (os.fstat(fd).st_dev, os.fstat(fd).st_ino)
        self.parent_identity = (os.fstat(parent).st_dev, os.fstat(parent).st_ino)

    def verify(self) -> None:
        """Repeat descriptor/path/content checks before each native effect stage."""
        parent = os.stat(self.path.parent, follow_symlinks=False)
        pinned_parent = os.fstat(self.parent)
        named = os.stat(self.path.name, dir_fd=self.parent, follow_symlinks=False)
        opened = os.fstat(self.fd)
        require(self.path.parent.resolve(strict=True) == self.path.parent
                and (parent.st_dev, parent.st_ino) == self.parent_identity
                and (pinned_parent.st_dev, pinned_parent.st_ino) == self.parent_identity
                and stat.S_ISDIR(parent.st_mode) and stat.S_IMODE(parent.st_mode) == 0o700
                and parent.st_uid in (0, os.geteuid()), 'intent parent custody changed')
        require(stat.S_ISREG(named.st_mode) and stat.S_ISREG(opened.st_mode)
                and stat.S_IMODE(named.st_mode) == stat.S_IMODE(opened.st_mode) == 0o600
                and named.st_uid == opened.st_uid == parent.st_uid
                and named.st_nlink == opened.st_nlink == 1
                and (named.st_dev, named.st_ino) == (opened.st_dev, opened.st_ino) == self.identity
                and opened.st_size == len(self.raw), 'intent file custody changed')
        os.lseek(self.fd, 0, os.SEEK_SET)
        data = bytearray()
        while len(data) <= len(self.raw):
            chunk = os.read(self.fd, len(self.raw) + 1 - len(data))
            if not chunk:
                break
            data += chunk
        after = os.fstat(self.fd)
        require(bytes(data) == self.raw and (after.st_mtime_ns, after.st_ctime_ns, after.st_size)
                == (opened.st_mtime_ns, opened.st_ctime_ns, opened.st_size), 'intent bytes changed')


def unique_object(pairs: list[tuple[str, object]]) -> dict:
    out = {}
    for key, value in pairs:
        require(key not in out, 'duplicate JSON key')
        out[key] = value
    return out


@contextmanager
def capsule(path: Path, new_intent: dict | None = None) -> Iterator[Capsule]:
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'intent custody requires POSIX no-follow descriptors')
    import fcntl
    require(path.is_absolute() and '..' not in path.parts and 1 <= len(str(path)) <= 4096
            and path.name not in ('', '.', '..'), 'intent path must be normalized and absolute')
    raw = None
    if new_intent is not None:
        verify_intent(new_intent)
        raw = canonical({'intent': new_intent, 'sha256': hashlib.sha256(canonical(new_intent)).hexdigest()}) + b'\n'
        require(len(raw) <= MAX_CAPSULE, 'intent capsule exceeds 64 KiB')
    parent = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    fd = None
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
            os.close(parent); parent = child
        directory = os.fstat(parent)
        require(stat.S_IMODE(directory.st_mode) == 0o700 and directory.st_uid in (0, os.geteuid()),
                'intent parent must be an owned exact-mode 0700 directory')
        flags = os.O_RDWR | os.O_CREAT | os.O_EXCL if new_intent is not None else os.O_RDONLY
        fd = os.open(path.name, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, 0o600, dir_fd=parent)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
                and before.st_uid == directory.st_uid and before.st_nlink == 1,
                'intent must be a private owned single-link regular file')
        if raw is not None:
            view = memoryview(raw)
            while view:
                count = os.write(fd, view); require(count > 0, 'short intent write'); view = view[count:]
            os.fsync(fd); os.fsync(parent)  # Both succeed BEFORE native preparation and dispatch.
            intent = new_intent
        else:
            require(1 <= before.st_size <= MAX_CAPSULE, 'empty or oversized intent; no repair performed')
            data = bytearray()
            while len(data) <= before.st_size:
                part = os.read(fd, before.st_size + 1 - len(data))
                if not part:
                    break
                data += part
            require(len(data) == before.st_size, 'intent extent changed')
            loaded = json.loads(data, object_pairs_hook=unique_object)
            require(isinstance(loaded, dict) and set(loaded) == {'intent', 'sha256'}, 'invalid intent envelope')
            intent = verify_intent(loaded['intent'])
            require(loaded['sha256'] == hashlib.sha256(canonical(intent)).hexdigest()
                    and bytes(data) == canonical(loaded) + b'\n', 'corrupt or noncanonical intent bytes')
            raw = bytes(data)
        owner = Capsule(path, fd, parent, raw, intent)
        owner.verify()
        yield owner
    finally:
        if fd is not None:
            os.close(fd)
        os.close(parent)



MAX_TERMINAL_RECEIPT = 2048


def terminal_name(owner: Capsule) -> str:
    # Content identity avoids filename-length problems and binds the *complete*
    # original capsule, including its endpoint, source and confirmation inputs.
    return '.dfmcp-dig-terminal-' + hashlib.sha256(owner.raw).hexdigest() + '.json'


def terminal_payload(owner: Capsule, raw: bytes) -> bytes:
    record = effect(raw, owner.intent)
    require(record['state'] in ('designated', 'refused'), 'only terminal native proof may be retained')
    payload = {'format': 'dfmcp.dig-terminal/1',
               'intent_sha256': hashlib.sha256(owner.raw).hexdigest(), 'effect_hex': raw.hex()}
    data = canonical({'receipt': payload, 'sha256': hashlib.sha256(canonical(payload)).hexdigest()}) + b'\n'
    require(len(data) <= MAX_TERMINAL_RECEIPT, 'terminal receipt exceeds its bound')
    return data


def verify_terminal(owner: Capsule, data: bytes) -> dict:
    require(1 <= len(data) <= MAX_TERMINAL_RECEIPT, 'empty or oversized terminal receipt')
    loaded = json.loads(data, object_pairs_hook=unique_object)
    require(isinstance(loaded, dict) and set(loaded) == {'receipt', 'sha256'}, 'invalid terminal envelope')
    payload = loaded['receipt']
    require(isinstance(payload, dict) and set(payload) == {'format', 'intent_sha256', 'effect_hex'}
            and payload['format'] == 'dfmcp.dig-terminal/1'
            and payload['intent_sha256'] == hashlib.sha256(owner.raw).hexdigest(),
            'terminal receipt belongs to a different intent')
    native = exact_hex(payload['effect_hex'], 207, 334)
    # Recompute the complete native proof, not just an outer JSON checksum.
    require(data == terminal_payload(owner, native), 'corrupt or noncanonical terminal receipt')
    return {'effect': effect(native, owner.intent),
            'terminal_receipt_sha256': hashlib.sha256(data).hexdigest()}


def terminal_receipt(owner: Capsule, native: bytes | None = None) -> dict | None:
    """Read or exclusively publish immutable proof under the existing intent lock.

    A failed/partial write is retained and refused, never repaired or overwritten.
    A complete write whose acknowledgement or sync was lost can be reverified and
    re-synced. Even offline acknowledgement syncs both descriptors; no native call,
    credential, receipt rewrite or renewed mutation authority is involved.
    """
    import fcntl
    owner.verify()
    expected = terminal_payload(owner, native) if native is not None else None
    name = terminal_name(owner)
    flags = os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
    created = False
    try:
        if expected is not None:
            try:
                fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL | flags, 0o600, dir_fd=owner.parent)
                created = True
            except FileExistsError:
                fd = os.open(name, os.O_RDONLY | flags, dir_fd=owner.parent)
        else:
            fd = os.open(name, os.O_RDONLY | flags, dir_fd=owner.parent)
    except FileNotFoundError:
        require(expected is None, 'terminal publication lost its parent')
        owner.verify()
        return None
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
                and before.st_uid == os.fstat(owner.parent).st_uid and before.st_nlink == 1,
                'terminal receipt must be a private owned single-link regular file')
        if created:
            view = memoryview(expected)
            while view:
                count = os.write(fd, view)
                require(count > 0, 'short terminal receipt write')
                view = view[count:]
            data = expected
        else:
            require(1 <= before.st_size <= MAX_TERMINAL_RECEIPT, 'empty or oversized terminal receipt; no repair')
            data = bytearray()
            while len(data) <= before.st_size:
                part = os.read(fd, before.st_size + 1 - len(data))
                if not part:
                    break
                data += part
            data = bytes(data)
            after = os.fstat(fd)
            require((before.st_mtime_ns, before.st_ctime_ns, before.st_size)
                    == (after.st_mtime_ns, after.st_ctime_ns, after.st_size)
                    and len(data) == before.st_size, 'terminal receipt changed during read')
            require(expected is None or data == expected, 'conflicting terminal evidence; never overwrite proof')
        result = verify_terminal(owner, data)
        pinned = Capsule(owner.path.parent / name, fd, owner.parent, data, owner.intent)
        pinned.verify(); owner.verify()
        os.fsync(fd); os.fsync(owner.parent)
        pinned.verify(); owner.verify()
        return result
    finally:
        os.close(fd)


def retained_result(owner: Capsule, receipt: dict) -> dict:
    result = recovered_result({'manifest': owner.intent['manifest'], 'effect': receipt['effect']}, False)
    result.update(receipt, terminal_receipt_retained=True, native_calls=0,
                  evidence_source='retained_terminal_receipt',
                  recovery='Retain intent and terminal receipt. Historical configuration proof is not current terrain or excavation completion.')
    return result


def finish_recovery(owner: Capsule, native: dict, dispatched: bool, replay: bool = False) -> dict:
    owner.verify()
    require(native['manifest'] == owner.intent['manifest'], 'receipt source differs from retained intent')
    record = effect(native['effect_raw'], owner.intent) if 'effect_raw' in native else None
    require(record == native.get('effect'), 'native effect presentation disagrees with proof')
    receipt = terminal_receipt(owner, native['effect_raw']) if record is not None and record['state'] in (
        'designated', 'refused') else terminal_receipt(owner)
    require(receipt is None or receipt['effect'] == record, 'native result regressed behind retained terminal proof')
    result = recovered_result(native, dispatched, replay)
    result.update(terminal_receipt_retained=receipt is not None, evidence_source='native_reply')
    if receipt is not None:
        result.update(receipt)
    owner.verify()
    return result


def observed_result(result: dict, allow_hidden: bool) -> dict:
    o = result['observation']
    preview = plan_for(o['region'], allow_hidden, bytes.fromhex(o['witness'])).hex()
    observed = {**o, 'cells': [{k: v for k, v in cell.items() if k != 'offset'} for cell in o['cells']]}
    return {'ok': True, 'profile': 'dig/1.16', 'read_only': True, 'manifest': result['manifest'],
            'observation': observed, 'allow_hidden_neighbors': allow_hidden,
            'blockers': blockers(o, allow_hidden), 'plan_digest_for_confirmation': preview,
            'excavation_safety_proven': False, 'game_mutation_dispatched': False}


def recovered_result(result: dict, dispatched: bool, prepared_replay: bool = False) -> dict:
    record = result.get('effect')
    return {'ok': True, 'profile': 'dig/1.16', 'manifest': result['manifest'], 'effect': record,
            'effect_status': record['state'] if record is not None else 'unknown',
            'commit_attempted_this_call': dispatched, 'native_preparation_replayed': prepared_replay,
            'absence_proves_non_application': False, 'retry_commit_permitted': False,
            'excavation_completion_proven': False,
            'recovery': 'Retain the intent. Query or cancel its exact native preparation; never repeat start for an uncertain effect.'}


def start(client: Client, path: Path, key: str, selected: dict, allow_hidden: bool,
          expected_witness: str, confirmed_plan: str) -> dict:
    key_bytes(key); region(selected); flag(allow_hidden)
    witness = exact_hex(expected_witness, 32); confirmed = exact_hex(confirmed_plan, 32)
    require(confirmed == plan_for(selected, allow_hidden, witness), 'confirmation differs from requested sealed plan')
    observed = client.observe(selected)
    require(hashlib.sha256(observed['raw']).digest() == witness, 'terrain changed since observation; no intent dispatched')
    intent = build_intent(client.address, key, selected, allow_hidden, observed['raw'], observed['manifest'])
    with capsule(path, intent) as owner:
        client.remaining(); owner.verify()
        retained = terminal_receipt(owner)
        if retained is not None:
            return retained_result(owner, retained)
        prepared = client.prepare(intent)
        if prepared['replayed']:
            return finish_recovery(owner, prepared, False, True)  # Never dispatch a replayed preparation.
        owner.verify(); client.remaining()
        committed = client.commit(intent, prepared)
        owner.verify()
        return finish_recovery(owner, committed, True)


def environment(control: bool, saved_endpoint: str | None = None) -> tuple[str, bytes]:
    require(os.environ.get('DFMCP_ALLOW_UNADMITTED_DIG_V1_16') == '1'
            and all(not k.startswith('DFMCP_') or k in ENVIRONMENT for k in os.environ),
            'dig client needs its exact development opt-in and refuses other DFMCP profile/admission state')
    production = os.environ.get('DFMCP_DIG_ALLOW_DESIGNATE')
    require(production in (None, '1') and (not control or production == '1'),
            'designation/cancellation requires separate operator designation enablement')
    configured = os.environ.get('DFMCP_DIG_ENDPOINT')
    require(saved_endpoint is None or configured is None or configured == saved_endpoint,
            'configured endpoint differs from retained intent')
    address = saved_endpoint or configured or '127.0.0.1:5000'; endpoint(address)
    token = os.environ.get('DFMCP_DIG_TOKEN', '').encode('utf-8')
    require(32 <= len(token) <= 256, 'missing or out-of-bound designation token')
    return address, token


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('observe', 'start', 'inspect', 'query', 'cancel'):
        p = sub.add_parser(name)
        if name in ('observe', 'start'):
            for k in REGION_KEYS:
                p.add_argument('--' + k, type=int, required=True)
            p.add_argument('--allow-hidden-neighbors', action='store_true')
        if name != 'observe':
            p.add_argument('--record', type=Path, required=True)
        if name != 'inspect':
            p.add_argument('--timeout-ms', type=int, default=10000)
        if name == 'start':
            p.add_argument('--key', required=True)
            p.add_argument('--expected-witness', required=True)
            p.add_argument('--confirm-plan', required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == 'inspect':  # Deliberately BEFORE any environment/token/client access.
            with capsule(args.record) as owner:
                retained = terminal_receipt(owner)
                result = retained_result(owner, retained) if retained is not None else {
                    'ok': True, 'profile': 'dig/1.16', 'effect_status': 'unknown', 'native_calls': 0,
                    'terminal_receipt_retained': False, 'retry_commit_permitted': False,
                    'excavation_completion_proven': False}
                result['intent'] = owner.intent
        elif args.command in ('query', 'cancel'):
            with capsule(args.record) as owner:
                retained = terminal_receipt(owner)
                if retained is not None:
                    result = retained_result(owner, retained)
                else:
                    address, token = environment(args.command == 'cancel', owner.intent['endpoint'])
                    with Client(address, token, args.timeout_ms) as client:
                        owner.verify()
                        native = client.cancel(owner.intent) if args.command == 'cancel' else client.query(owner.intent)
                        result = finish_recovery(owner, native, False)
        else:
            selected = {k: getattr(args, k) for k in REGION_KEYS}; region(selected)
            address, token = environment(args.command == 'start')
            with Client(address, token, args.timeout_ms) as client:
                result = observed_result(client.observe(selected), args.allow_hidden_neighbors) if args.command == 'observe' else start(
                    client, args.record, args.key, selected, args.allow_hidden_neighbors, args.expected_witness, args.confirm_plan)
        data = canonical(result)
        require(len(data) <= MAX_OUTPUT, 'complete developer response exceeds 128 KiB; query retained intent')
        print(data.decode('ascii')); return 0
    except (Rejected, OSError, ValueError, KeyError, TypeError, struct.error, RecursionError):
        print(canonical({'ok': False, 'profile': 'dig/1.16', 'effect_status': 'unknown',
            'error': 'Request, custody, source, deadline or evidence validation failed. Retain intent; do not retry designation.',
            'retry_commit_permitted': False, 'excavation_completion_proven': False}).decode('ascii'))
        return 2


if __name__ == '__main__':
    sys.exit(main())
