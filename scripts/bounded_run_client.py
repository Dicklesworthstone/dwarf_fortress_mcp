#!/usr/bin/env python3
"""One-shot POSIX developer client for the isolated native run/1.13 profile.

Not an MCP server or production admission. Start writes and syncs a new immutable
intent capsule BEFORE Prepare/Commit. Existing capsules support only inspect,
query, and cancel: there is deliberately no resume/replay-commit operation.
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

PROFILE = 'run/1.13'
METHODS = ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun', 'CancelRun')
PHASES = ('prepared', 'running', 'stopping', 'stopped', 'refused', 'source_lost')
REASONS = ('none', 'tick_limit', 'wall_limit', 'cancelled', 'external_pause',
           'native_failure', 'clock_regression', 'source_changed', 'shutdown', 'stale')
MAX_TICK = (2**32 - 1) * 403200 + 403199
INTENT_KEYS = {'format', 'endpoint', 'idempotency_key', 'game_ticks', 'wall_ms', 'observation_hex',
               'plan_digest_hex', 'prepare_token_hex', 'df_version', 'dfhack_version', 'effect_status'}


class Rejected(ValueError):
    """Closed protocol, budget, or custody refusal; never an effect outcome."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Rejected(message)


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'integer outside profile bounds')
    return value  # type: ignore[return-value]


def key_bytes(key: str) -> bytes:
    require(isinstance(key, str) and re.fullmatch(r'[A-Za-z0-9_.-]{1,128}', key) is not None,
            'invalid idempotency key')
    raw = key.encode('ascii')
    return struct.pack('>H', len(raw)) + raw


def digest(domain: bytes, data: bytes) -> bytes:
    return hashlib.sha256(domain + b'\0' + data).digest()


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def exact_hex(value: object, size: int) -> bytes:
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{' + str(size * 2) + '}', value) is not None,
            'noncanonical hexadecimal field')
    return bytes.fromhex(value)  # type: ignore[arg-type]


def text(raw: object) -> str:
    require(isinstance(raw, bytes) and 1 <= len(raw) <= 128, 'invalid version field')
    value = raw.decode('utf-8')
    require(all(ord(c) >= 32 and ord(c) != 127 for c in value), 'control character in version')
    return value


def snapshot(raw: bytes) -> dict:
    require(len(raw) == 35 and raw[:8] == b'DFMRO013', 'invalid clock observation')
    generation, sequence, tick, loaded, valid, paused = struct.unpack('>QQQBBB', raw[8:])
    integer(generation, 1, 2**64 - 2)
    require(all(flag in (0, 1) for flag in (loaded, valid, paused)), 'invalid observation boolean')
    require(loaded or not (valid or paused), 'unloaded clock claims live fields')
    require((valid and tick <= MAX_TICK) or (not valid and tick == 0), 'invalid observed game tick')
    return {'generation': generation, 'sequence': sequence, 'tick': tick,
            'loaded': bool(loaded), 'clock_valid': bool(valid), 'paused': bool(paused)}


def plan_for(ticks: int, wall_ms: int, observation: bytes) -> bytes:
    integer(ticks, 1, 1200); integer(wall_ms, 1, 60000); snapshot(observation)
    return digest(b'dfmcp-bounded-run-plan/1', struct.pack('>II', ticks, wall_ms) + observation)


def token_for(key: str, plan: bytes) -> bytes:
    require(len(plan) == 32, 'invalid plan digest')
    return digest(b'dfmcp-bounded-run-token/1', key_bytes(key) + plan)[:16]


def decode_record(raw: bytes) -> dict:
    require(147 <= len(raw) <= 274 and raw[:8] == b'DFMRE013', 'invalid run record')
    width = struct.unpack('>H', raw[8:10])[0]
    require(1 <= width <= 128 and len(raw) == 146 + width, 'invalid record key length')
    key = raw[10:10 + width].decode('ascii'); key_bytes(key)
    offset = 10 + width
    ticks, wall_ms = struct.unpack('>II', raw[offset:offset + 8]); offset += 8
    observation = raw[offset:offset + 35]; offset += 35
    before = snapshot(observation)
    require(before['loaded'] and before['clock_valid'] and before['paused'], 'record has no eligible precondition')
    plan = raw[offset:offset + 32]; offset += 32
    token = raw[offset:offset + 16]; offset += 16
    phase, reason, attempted, verified, known = raw[offset:offset + 5]; offset += 5
    tick = struct.unpack('>Q', raw[offset:offset + 8])[0]
    require(plan == plan_for(ticks, wall_ms, observation) and token == token_for(key, plan), 'record plan/token mismatch')
    require(raw[-32:] == digest(b'dfmcp-bounded-run-receipt/1', raw[:-32]), 'record integrity mismatch')
    require(phase < len(PHASES) and reason < len(REASONS) and all(v in (0, 1) for v in (attempted, verified, known)),
            'invalid record state code')
    require((known and tick <= MAX_TICK) or (not known and tick == 0), 'unknown tick encoded as observed')
    require(bool(attempted) == (phase in (1, 2, 3, 5)) and bool(verified) == (phase == 3), 'impossible effect flags')
    if phase == 0:
        require(reason == 0 and not known, 'invalid prepared record')
    elif phase == 1:
        require(reason == 0 and known and tick >= before['tick'], 'invalid running record')
    elif phase in (2, 3):
        require(reason in ((1, 2, 3, 5, 6, 8) if phase == 2 else (1, 2, 3, 4, 5, 6, 8)), 'invalid stop reason')
    elif phase == 4:
        require(reason in (3, 7, 9) and not known, 'invalid refused record')
    else:
        require(reason == 7 and not known, 'invalid source-loss record')
    advanced = tick - before['tick'] if known and tick >= before['tick'] else None
    return {'key': key, 'game_ticks': ticks, 'wall_ms': wall_ms, 'before': before,
            'observation_hex': observation.hex(), 'plan_digest_hex': plan.hex(), 'prepare_token_hex': token.hex(),
            'phase': PHASES[phase], 'reason': REASONS[reason], 'unpause_attempted': bool(attempted),
            'pause_verified': bool(verified), 'observed_tick': tick if known else None,
            'observed_ticks_advanced': advanced, 'observed_tick_overshoot': max(0, advanced - ticks) if advanced is not None else None,
            'receipt_digest_hex': raw[-32:].hex(), 'current_pause_unproved': True}


def varint(value: int) -> bytes:
    integer(value, 0, 2**64 - 1)
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128); value >>= 7
    out.append(value)
    return bytes(out)


def encode(fields: dict[int, int | bytes]) -> bytes:
    out = bytearray()
    for field, value in sorted(fields.items()):
        integer(field, 1, 12)
        if isinstance(value, bytes):
            out += varint(field * 8 + 2) + varint(len(value)) + value
        else:
            out += varint(field * 8) + varint(value)
    require(len(out) <= 2048, 'request exceeds 2 KiB')
    return bytes(out)


def decode(raw: bytes, maximum: int = 12) -> dict:
    require(len(raw) <= 2048, 'reply exceeds 2 KiB')
    offset, fields = 0, {}

    def number() -> int:
        nonlocal offset
        result = 0
        for index in range(10):
            require(offset < len(raw), 'truncated varint')
            byte = raw[offset]; offset += 1
            require(index < 9 or byte <= 1, 'varint overflow')
            result |= (byte & 127) << (index * 7)
            if byte < 128:
                require(index == 0 or byte != 0, 'noncanonical varint')
                return result
        raise Rejected('unterminated varint')

    while offset < len(raw):
        tag = number(); field, wire_type = tag >> 3, tag & 7
        require(1 <= field <= maximum and field not in fields, 'unknown/duplicate protobuf field')
        if wire_type == 0:
            value = number()
        elif wire_type == 2:
            width = number(); require(width <= len(raw) - offset, 'truncated byte field')
            value = raw[offset:offset + width]; offset += width
        else:
            raise Rejected('unsupported protobuf wire type')
        fields[field] = value
    return fields


def endpoint(raw: str) -> tuple[str, int]:
    match = re.fullmatch(r'([0-9.]+):([0-9]{1,5})', raw)
    require(match is not None, 'endpoint must be a numeric IPv4 loopback address and port')
    address = ipaddress.IPv4Address(match[1])
    require(address.is_loopback, 'unencrypted developer RPC is loopback-only')
    return str(address), integer(int(match[2]), 1, 65535)


class Client:
    """One foreground connection, one non-renewable deadline, no reconnect/replay."""
    def __init__(self, address: tuple[str, int], token: bytes, timeout_ms: int):
        integer(timeout_ms, 1, 60000)
        require(32 <= len(token) <= 256, 'invalid developer credential length')
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.token, self.nonce = token, secrets.token_bytes(32)
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.manifest = None
        self.methods = {}
        try:
            self.sock.settimeout(self.remaining()); self.sock.connect(address)
            self.send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self.read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'invalid DFHack handshake')
            for name in METHODS:
                request = encode({1: name.encode(), 2: b'dfmcp.run.v1_13.Request',
                                  3: b'dfmcp.run.v1_13.Reply', 4: b'dfmcp_run_v1_13'})
                bound = decode(self.frame(0, request), 1)
                require(set(bound) == {1}, 'invalid method binding')
                method = integer(bound[1], 2, 32767)
                require(method not in self.methods.values(), 'native method IDs alias')
                self.methods[name] = method
            self.call('Handshake')
        except BaseException:
            self.close()
            raise

    def remaining(self) -> float:
        value = self.deadline - time.monotonic()
        require(value >= 0.001, 'shared RPC deadline exhausted')
        return value

    def send(self, data: bytes) -> None:
        self.sock.settimeout(self.remaining()); self.sock.sendall(data)

    def read(self, size: int) -> bytes:
        out = bytearray()
        while len(out) < size:
            self.sock.settimeout(self.remaining())
            part = self.sock.recv(size - len(out))
            require(bool(part), 'native connection closed before complete reply')
            out += part
        return bytes(out)

    def frame(self, method: int, request: bytes) -> bytes:
        require(len(request) <= 2048, 'request exceeds 2 KiB')
        self.send(struct.pack('<h2xi', method, len(request)) + request)
        notifications, total = 0, 0
        while True:
            message, length = struct.unpack('<h2xi', self.read(8))
            require(message in (-1, -3), 'DFHack rejected RPC or emitted an unknown frame')
            require(0 <= length <= (2048 if message == -1 else 65536), 'native frame length refused')
            if message == -3:
                notifications += 1; total += length
                require(notifications <= 8 and total <= 262144, 'native notification budget exhausted')
            payload = self.read(length)
            if message == -1:
                return payload

    def call(self, operation: str, fields: dict | None = None) -> dict:
        require(operation in METHODS, 'operation outside fixed run profile')
        fields = fields or {}
        require(all(type(k) is int and 5 <= k <= 10 for k in fields), 'operation cannot override credentials/profile')
        try:
            response = decode(self.frame(self.methods[operation], encode({1: self.token, 2: self.nonce, 3: 1, 4: 13, **fields})))
            require(set(range(1, 9)) <= set(response), 'required reply fields missing')
            require(response[3] == self.nonce and response[4] == 1 and response[5] == 13, 'reply nonce/profile mismatch')
            accepted = integer(response[1], 0, 1); code = integer(response[2], 0, 8)
            if not accepted:
                require(code != 0 and set(response) == set(range(1, 9)), 'invalid failure reply')
                raise Rejected(f'native request refused (code {code}); no effect outcome inferred')
            require(code == 0 and {11, 12} <= set(response), 'invalid success reply')
            current = {'generation': integer(response[6], 1, 2**64 - 2),
                       'df_version': text(response[7]), 'dfhack_version': text(response[8])}
            if self.manifest is not None:
                require(current['generation'] >= self.manifest['generation'] and
                        all(current[k] == self.manifest[k] for k in ('df_version', 'dfhack_version')),
                        'native identity changed or generation regressed')
            self.manifest = current
            result = {'manifest': current, 'owner_active': bool(integer(response[11], 0, 1)),
                      'retained_records': integer(response[12], 0, 256)}
            require((9 in response) == (operation == 'ObserveRun'), 'unexpected/missing observation')
            require(operation == 'QueryRun' or (10 in response) == (operation in ('PrepareRun', 'CommitRun', 'CancelRun')),
                    'unexpected/missing effect record')
            if 9 in response:
                require(isinstance(response[9], bytes), 'invalid observation wire type')
                observed = snapshot(response[9])
                require(observed['generation'] == current['generation'], 'observation source mismatch')
                result.update(observation_hex=response[9].hex(), observation=observed)
            if 10 in response:
                require(isinstance(response[10], bytes), 'invalid record wire type')
                record = decode_record(response[10])
                require(record['key'].encode() == fields.get(5) and bytes.fromhex(record['plan_digest_hex']) == fields.get(9),
                        'reply record belongs to another intent')
                require(record['before']['generation'] <= current['generation'], 'record from a future source')
                if record['phase'] in ('running', 'stopping'):
                    require(record['before']['generation'] == current['generation'] and result['owner_active'], 'unowned running record')
                result['record'] = record
            return result
        except BaseException:
            self.close()  # Unread frame bytes can never become another reply.
            raise

    def close(self) -> None:
        self.sock.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_args) -> None:
        self.close()


def verify_intent(value: object) -> dict:
    require(isinstance(value, dict) and set(value) == INTENT_KEYS, 'invalid intent capsule fields')
    require(value['format'] == 'dfmcp.bounded-run-intent/1' and value['effect_status'] == 'indeterminate_until_native_query',
            'intent is not a completion receipt')
    address = endpoint(value['endpoint']); require(f'{address[0]}:{address[1]}' == value['endpoint'], 'noncanonical endpoint')
    key_bytes(value['idempotency_key'])
    raw = exact_hex(value['observation_hex'], 35)
    before = snapshot(raw); require(before['loaded'] and before['clock_valid'] and before['paused'], 'ineligible run precondition')
    plan = plan_for(value['game_ticks'], value['wall_ms'], raw)
    require(exact_hex(value['plan_digest_hex'], 32) == plan and
            exact_hex(value['prepare_token_hex'], 16) == token_for(value['idempotency_key'], plan), 'intent commitment mismatch')
    for field in ('df_version', 'dfhack_version'):
        require(isinstance(value[field], str), 'invalid intent version')
        text(value[field].encode('utf-8'))
    return value


@contextmanager
def capsule(path: Path, new_intent: dict | None = None) -> Iterator[dict]:
    """Pinned, non-symlink POSIX custody. New files only; never repair or overwrite."""
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'private capsule custody requires POSIX O_NOFOLLOW')
    import fcntl
    require(path.is_absolute() and '..' not in path.parts and len(str(path)) <= 4096, 'record path must be absolute without parent traversal')
    parent = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    fd = None
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
            os.close(parent); parent = child
        info = os.fstat(parent)
        require(stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid in (0, os.geteuid()), 'record parent must be owner-private mode 0700')
        flags = (os.O_WRONLY | os.O_CREAT | os.O_EXCL) if new_intent is not None else os.O_RDONLY
        fd = os.open(path.name, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, 0o600, dir_fd=parent)
        fcntl.flock(fd, (fcntl.LOCK_EX if new_intent is not None else fcntl.LOCK_SH) | fcntl.LOCK_NB)
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
                and before.st_uid in (0, os.geteuid()) and before.st_nlink == 1, 'record must be a private regular single-link file')
        if new_intent is not None:
            intent = verify_intent(new_intent)
            data = canonical({'intent': intent, 'sha256': hashlib.sha256(canonical(intent)).hexdigest()}) + b'\n'
            require(len(data) <= 8192, 'intent capsule too large')
            remaining = memoryview(data)
            while remaining:
                count = os.write(fd, remaining); require(count > 0, 'short intent write'); remaining = remaining[count:]
            os.fsync(fd); os.fsync(parent)  # Both precede native preparation/commit.
        else:
            require(1 <= before.st_size <= 8192, 'empty or oversized intent capsule')
            data = bytearray()
            while len(data) <= before.st_size:
                chunk = os.read(fd, before.st_size + 1 - len(data))
                if not chunk:
                    break
                data += chunk
            require(len(data) == before.st_size, 'intent changed during read')
            loaded = json.loads(data)
            require(isinstance(loaded, dict) and set(loaded) == {'intent', 'sha256'}, 'invalid capsule envelope')
            intent = verify_intent(loaded['intent'])
            require(loaded['sha256'] == hashlib.sha256(canonical(intent)).hexdigest()
                    and bytes(data) == canonical(loaded) + b'\n', 'intent bytes are not canonical or intact')
        after = os.fstat(fd); named = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
        require(after.st_size == len(data) and (after.st_dev, after.st_ino) == (named.st_dev, named.st_ino)
                and after.st_nlink == 1, 'intent custody changed')
        if new_intent is None:
            require((before.st_mtime_ns, before.st_ctime_ns) == (after.st_mtime_ns, after.st_ctime_ns), 'intent changed during read')
        yield intent
    finally:
        if fd is not None:
            os.close(fd)
        os.close(parent)


def intent_fields(intent: dict, with_token: bool = False) -> dict:
    result = {5: intent['idempotency_key'].encode(), 9: bytes.fromhex(intent['plan_digest_hex'])}
    if with_token:
        result[10] = bytes.fromhex(intent['prepare_token_hex'])
    return result


def match_record(result: dict, intent: dict) -> None:
    require(all(result['manifest'][k] == intent[k] for k in ('df_version', 'dfhack_version')), 'intent software tuple changed')
    record = result.get('record')
    if record is not None:
        require(record['key'] == intent['idempotency_key'] and all(record[k] == intent[k] for k in
                ('game_ticks', 'wall_ms', 'observation_hex', 'plan_digest_hex', 'prepare_token_hex')), 'record does not match durable intent')


def start(client: Client, address: tuple[str, int], path: Path, key: str, ticks: int, wall_ms: int) -> dict:
    key_bytes(key); integer(ticks, 1, 1200); integer(wall_ms, 1, 60000)
    observed = client.call('ObserveRun'); raw = bytes.fromhex(observed['observation_hex'])
    plan = plan_for(ticks, wall_ms, raw)
    intent = {'format': 'dfmcp.bounded-run-intent/1', 'endpoint': f'{address[0]}:{address[1]}',
              'idempotency_key': key, 'game_ticks': ticks, 'wall_ms': wall_ms,
              'observation_hex': raw.hex(), 'plan_digest_hex': plan.hex(), 'prepare_token_hex': token_for(key, plan).hex(),
              'df_version': observed['manifest']['df_version'], 'dfhack_version': observed['manifest']['dfhack_version'],
              'effect_status': 'indeterminate_until_native_query'}
    with capsule(path, intent):
        prepared = client.call('PrepareRun', {**intent_fields(intent), 6: ticks, 7: wall_ms, 8: raw})
        match_record(prepared, intent)
        require(prepared.get('record', {}).get('phase') == 'prepared', 'existing non-prepared run is not eligible for start')
        result = client.call('CommitRun', intent_fields(intent, True))  # Exactly one attempt, no replay loop.
        match_record(result, intent)
        return result


def recover(client: Client, intent: dict, cancel: bool = False, wait_ms: int = 0) -> dict:
    integer(wait_ms, 0, 60000)
    end = min(client.deadline, time.monotonic() + wait_ms / 1000)
    result = client.call('CancelRun' if cancel else 'QueryRun', intent_fields(intent, cancel))
    match_record(result, intent)
    # Only QueryRun repeats. Cancellation and an old unpause are never replayed.
    while not cancel and wait_ms and result.get('record', {}).get('phase') in ('running', 'stopping'):
        remaining = end - time.monotonic()
        if remaining <= 0.101:
            result['wait_expired'] = True; break
        time.sleep(min(0.1, remaining))
        result = client.call('QueryRun', intent_fields(intent)); match_record(result, intent)
    result['source_changed_since_intent'] = result['manifest']['generation'] != snapshot(bytes.fromhex(intent['observation_hex']))['generation']
    if 'record' not in result:
        result['effect_status'] = 'unknown_absent_record_is_not_nonapplication'
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('observe', 'start', 'inspect', 'query', 'cancel'))
    parser.add_argument('--record', type=Path)
    parser.add_argument('--key')
    parser.add_argument('--ticks', type=int)
    parser.add_argument('--wall-ms', type=int)
    parser.add_argument('--timeout-ms', type=int, default=10000)
    parser.add_argument('--wait-ms', type=int, default=0, help='foreground query-only wait, at most 60000 ms')
    args = parser.parse_args(argv)
    try:
        require(args.operation == 'observe' or args.record is not None, '--record is required')
        require(args.operation == 'start' or all(v is None for v in (args.key, args.ticks, args.wall_ms)), 'start-only fields supplied')
        require(args.operation == 'query' or args.wait_ms == 0, '--wait-ms belongs only to query')
        if args.operation == 'inspect':
            with capsule(args.record) as intent:
                result = {'intent': intent, 'evidence_scope': 'recorded_intent_only', 'effect_status': 'unknown', 'native_contacted': False}
        else:
            require(os.environ.get('DFMCP_ALLOW_UNADMITTED_RUN_V1_13') == '1'
                    and 'DFMCP_ADMITTED_BRIDGE_PROTOCOL' not in os.environ, 'exact unadmitted developer opt-in required')
            address = endpoint(os.environ.get('DFMCP_RUN_ENDPOINT', '127.0.0.1:5000'))
            token = os.environ.get('DFMCP_RUN_TOKEN', '').encode('utf-8')
            if args.operation in ('query', 'cancel'):
                # Resolve private custody and endpoint before contacting a native source.
                with capsule(args.record) as intent:
                    require(intent['endpoint'] == f'{address[0]}:{address[1]}', 'operator endpoint differs from recorded intent')
                    with Client(address, token, args.timeout_ms) as client:
                        result = recover(client, intent, args.operation == 'cancel', args.wait_ms)
            else:
                if args.operation == 'start':
                    key_bytes(args.key); integer(args.ticks, 1, 1200); integer(args.wall_ms, 1, 60000)
                with Client(address, token, args.timeout_ms) as client:
                    result = (client.call('ObserveRun') if args.operation == 'observe'
                              else start(client, address, args.record, args.key, args.ticks, args.wall_ms))
        print(json.dumps({'ok': True, 'profile': PROFILE, 'runtime_admitted': False, 'result': result}, sort_keys=True))
        return 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError) as error:
        # No raw frames, credentials, or server-controlled error strings are printed.
        print(json.dumps({'ok': False, 'profile': PROFILE, 'runtime_admitted': False,
                          'error_class': type(error).__name__, 'effect_status': 'unknown',
                          'detail': str(error) if isinstance(error, Rejected) else 'I/O, custody, or decoding failed; intent was not replayed'}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
