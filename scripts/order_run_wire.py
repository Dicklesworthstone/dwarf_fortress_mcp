"""Closed order-run/1.14 developer wire; checksums are evidence, never authority."""
from __future__ import annotations

import hashlib
import ipaddress
import re
import secrets
import socket
import struct
import time

METHODS = ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun', 'QueryRun', 'CancelRun')
PHASES = ('prepared', 'running', 'stopping', 'stopped', 'refused', 'source_lost')
REASONS = ('none', 'tick_limit', 'wall_limit', 'cancelled', 'external_pause',
           'native_failure', 'clock_regression', 'source_changed', 'shutdown', 'stale')
TRIGGERS = ('none', 'predicate_observed', 'target_absent', 'target_changed',
            'counter_regression', 'horizon_regression', 'source_changed', 'native_failure')
MAX_TICK = (2**32 - 1) * 403200 + 403199


class Rejected(ValueError):
    """A bounded protocol refusal, not proof that an effect did not occur."""


def require(value, message):
    if not value:
        raise Rejected(message)


def integer(value, low, high):
    require(type(value) is int and low <= value <= high, 'integer outside profile bounds')
    return value


def text(value, maximum=128):
    require(type(value) is str and 1 <= len(value.encode('utf-8')) <= maximum and '\0' not in value,
            'invalid bounded UTF-8 text')
    return value


def key(value):
    require(type(value) is str and re.fullmatch(r'[A-Za-z0-9_.-]{1,128}', value), 'invalid operation key')
    return value


def unhex(value, minimum, maximum=None):
    maximum = minimum if maximum is None else maximum
    require(type(value) is str and minimum * 2 <= len(value) <= maximum * 2 and len(value) % 2 == 0
            and re.fullmatch('[0-9a-f]+', value), 'invalid canonical hex bytes')
    return bytes.fromhex(value)


def hashed(domain, data):
    return hashlib.sha256(domain + b'\0' + data).digest()


def field(data):
    require(len(data) <= 65535, 'field too large')
    return struct.pack('>H', len(data)) + data


class Reader:
    def __init__(self, data):
        require(type(data) is bytes, 'wire payload is not bytes')
        self.data, self.offset = data, 0

    def take(self, length):
        require(0 <= length <= len(self.data) - self.offset, 'truncated record')
        out = self.data[self.offset:self.offset + length]; self.offset += length
        return out

    def number(self, size, signed=False):
        return int.from_bytes(self.take(size), 'big', signed=signed)

    def field(self, maximum):
        length = self.number(2); require(length <= maximum, 'oversized field')
        return self.take(length)

    def end(self):
        require(self.offset == len(self.data), 'trailing bytes')


def capture(data):
    require(62 <= len(data) <= 573, 'capture byte bound')
    r = Reader(data); require(r.take(8) == b'DFMOR014', 'wrong capture generation')
    generation, sequence, tick = r.number(8), r.number(8), r.number(8)
    site = r.number(4); folder = text(r.field(512).decode('utf-8'), 512)
    paused, target, horizon, present, recipe = r.number(1), r.number(4), r.number(4), r.number(1), r.number(1)
    total, left, status = r.number(4, True), r.number(4, True), r.number(4); r.end()
    integer(generation, 1, 2**64 - 2); integer(tick, 0, MAX_TICK)
    for n in (site, target, horizon):
        integer(n, 0, 2**31 - 1)
    require(paused in (0, 1) and present in (0, 1) and recipe <= 4, 'invalid capture flags')
    if not present:
        require((recipe, total, left, status) == (0, 0, 0, 0), 'absent target has backing values')
    else:
        require(target < horizon and -32768 <= total <= 32767 and -32768 <= left <= 32767, 'invalid order counters')
        if recipe:
            require(1 <= total <= 100 and 0 <= left <= total and status & ~3 == 0, 'unrecognized finite template')
    return dict(generation=generation, sequence=sequence, tick=tick, site=site, folder=folder,
                paused=bool(paused), order_id=target, horizon=horizon, present=bool(present),
                recipe=recipe, total=total, remaining=left, status=status)


def matches(condition, value):
    if not value['present'] or not value['recipe']:
        return False
    predicate = condition['predicate']
    return (bool(value['status'] & 1) if predicate == 1 else bool(value['status'] & 2) if predicate == 2
            else value['remaining'] <= condition['threshold'])


def make_plan(before, ticks, wall, predicate, threshold=0, samples=1, interval=1):
    value = capture(before)
    integer(ticks, 1, 1200); integer(wall, 1, 60000); integer(predicate, 1, 3)
    integer(samples, 1, 16); integer(interval, 1, 1200); integer(threshold, 0, 100)
    require(samples * interval <= ticks, 'sample cadence exceeds run horizon')
    require(value['paused'] and value['present'] and value['recipe'] and value['sequence'] < 2**64 - 1,
            'run requires an eligible paused finite order')
    require(threshold <= value['total'] and (predicate == 3 or threshold == 0), 'invalid predicate threshold')
    require(not matches(dict(predicate=predicate, threshold=threshold), value), 'condition already true; do not unpause')
    return b'DFMOP014' + struct.pack('>IIBIII', ticks, wall, predicate, threshold, samples, interval) + field(before)


def plan(data):
    require(93 <= len(data) <= 604, 'plan byte bound')
    r = Reader(data); require(r.take(8) == b'DFMOP014', 'wrong plan generation')
    ticks, wall, predicate = r.number(4), r.number(4), r.number(1)
    threshold, samples, interval = r.number(4), r.number(4), r.number(4)
    before = r.field(573); r.end()
    require(make_plan(before, ticks, wall, predicate, threshold, samples, interval) == data, 'noncanonical plan')
    return dict(game_ticks=ticks, wall_ms=wall, predicate=predicate, threshold=threshold,
                samples=samples, interval=interval, before=capture(before), capture_hex=before.hex())


def plan_digest(data):
    plan(data)
    return hashed(b'dfmcp-order-run-plan/1', data)


def token(operation_key, digest):
    require(len(digest) == 32, 'invalid plan digest width')
    return hashed(b'dfmcp-order-run-token/1', field(key(operation_key).encode('ascii')) + digest)[:16]


def record(data):
    require(214 <= len(data) <= 1425, 'record byte bound')
    r = Reader(data); require(r.take(8) == b'DFMOE014', 'wrong record generation')
    operation_key = key(r.field(128).decode('ascii')); encoded_plan = r.field(604); goal = plan(encoded_plan)
    digest, prepared = r.take(32), r.take(16)
    phase, reason, trigger, attempted, verified, known = (r.number(1) for _ in range(6))
    observed, count, counted = r.number(8), r.number(4), r.number(8)
    sample_bytes = r.field(573); receipt = r.take(32); r.end()
    require(digest == plan_digest(encoded_plan) and prepared == token(operation_key, digest), 'record identity mismatch')
    require(receipt == hashed(b'dfmcp-order-run-receipt/1', data[:-32]), 'receipt hash mismatch')
    require(phase < 6 and reason < 10 and trigger < 8 and all(v in (0, 1) for v in (attempted, verified, known)),
            'record state code out of bounds')
    require(bool(attempted) == (phase in (1, 2, 3, 5)) and bool(verified) == (phase == 3), 'impossible effect flags')
    require((known and observed <= MAX_TICK) or (not known and observed == 0), 'unknown tick has nonzero backing')
    before = goal['before']; sample = capture(sample_bytes) if sample_bytes else None
    require(count <= goal['samples'] and before['tick'] + count * goal['interval'] <= counted <= MAX_TICK, 'invalid stability evidence')
    if sample:
        require(all(sample[f] == before[f] for f in ('generation', 'folder', 'site', 'order_id'))
                and sample['sequence'] == before['sequence'] + 1 and not sample['paused']
                and before['tick'] <= counted <= sample['tick'] < before['tick'] + goal['game_ticks'], 'sample outside sealed run')
    else:
        require(count == 0 and counted == before['tick'], 'stability without a sample')
    if phase in (0, 4):
        require(not known and not trigger and sample is None and reason in ((0,) if phase == 0 else (3, 7, 9)),
                'undispatched record claims observation')
    elif phase == 1:
        require(reason == 0 and trigger == 0 and known and observed >= before['tick'] and count < goal['samples'], 'invalid running state')
    elif phase in (2, 3):
        allowed = (1, 2, 3, 5, 6, 8) if phase == 2 else (1, 2, 3, 4, 5, 6, 8)
        require(reason in allowed and trigger != 6, 'invalid stopping reason')
    else:
        require(reason == 7 and not known, 'source loss cannot prove current clock')
    if 1 <= trigger <= 5:
        require(phase in (2, 3, 5) and sample is not None and (phase == 5 or reason == 3), 'trigger lacks stop request/sample')
    if trigger == 1:
        require(count == goal['samples'] and counted == sample['tick'] and matches(goal, sample)
                and sample['recipe'] == before['recipe'] and sample['total'] == before['total']
                and sample['remaining'] <= before['remaining'] and sample['horizon'] >= before['horizon'], 'predicate evidence mismatch')
    else:
        require(count < goal['samples'], 'unclaimed predicate has terminal count')
    if trigger == 2:
        require(not sample['present'], 'absence trigger has present target')
    if trigger == 3:
        require(sample['present'] and (sample['recipe'] == 0 or sample['recipe'] != before['recipe']
                or sample['total'] != before['total']), 'configuration trigger has unchanged template')
    if trigger in (4, 5):
        # Previous callback values are not retained in this protocol; never
        # pretend the last sample alone proves the reported regression.
        require(sample is not None, 'regression trigger has no sample')
    if trigger == 6:
        require(phase == 5, 'source change trigger without source loss')
    if trigger == 7:
        require(phase in (2, 3, 5), 'native failure trigger is not stopping')
    advanced = observed - before['tick'] if known and observed >= before['tick'] else None
    return dict(key=operation_key, plan_hex=encoded_plan.hex(), plan_digest=digest.hex(), goal=goal,
                phase=PHASES[phase], reason=REASONS[reason], trigger=TRIGGERS[trigger],
                unpause_attempted=bool(attempted), pause_verified=bool(verified), observed_tick=observed if known else None,
                reported_stable_samples=count, counted_tick=counted, sample=sample,
                predicate_observed=trigger == 1, observed_tick_overshoot=max(0, advanced - goal['game_ticks']) if advanced is not None else None,
                receipt_digest=receipt.hex(), goods_produced_proven=False, current_pause_unproved=True,
                full_sample_trace_available=False)


def address(raw):
    require(type(raw) is str and re.fullmatch(r'[0-9.]+:[0-9]{1,5}', raw), 'numeric IPv4 loopback endpoint required')
    host, port = raw.split(':'); host = ipaddress.IPv4Address(host); port = integer(int(port), 1, 65535)
    require(host.is_loopback and raw == f'{host}:{port}', 'canonical loopback endpoint required')
    return str(host), port


def varint(n):
    integer(n, 0, 2**64 - 1); out = bytearray()
    while n >= 128:
        out.append((n & 127) | 128); n >>= 7
    return bytes(out + bytes([n]))


def encode(fields):
    out = b''
    for tag, value in sorted(fields.items()):
        integer(tag, 1, 15)
        out += (varint(tag * 8 + 2) + varint(len(value)) + value) if type(value) is bytes else (varint(tag * 8) + varint(value))
    require(len(out) <= 2048, 'request exceeds 2 KiB')
    return out


def decode(data, maximum=12):
    require(len(data) <= 4096, 'reply exceeds 4 KiB'); r = Reader(data); values = {}
    def number():
        result = 0
        for i in range(10):
            b = r.number(1); require(i < 9 or b <= 1, 'varint overflow'); result |= (b & 127) << (7 * i)
            if b < 128:
                require(i == 0 or b != 0, 'overlong varint'); return result
        raise Rejected('unterminated varint')
    while r.offset < len(data):
        tag = number(); name, kind = tag >> 3, tag & 7
        require(1 <= name <= maximum and name not in values and kind in (0, 2), 'unknown/duplicate protobuf field')
        values[name] = number() if kind == 0 else r.take(number())
    return values


class Client:
    """One foreground socket and one absolute deadline; no reconnect/retry."""
    def __init__(self, endpoint, secret, timeout_ms):
        self.endpoint = endpoint; target = address(endpoint)
        integer(timeout_ms, 1, 60000); require(type(secret) is bytes and 32 <= len(secret) <= 256, 'credential length invalid')
        self.deadline = time.monotonic() + timeout_ms / 1000
        self.secret, self.nonce, self.manifest, self.methods = secret, secrets.token_bytes(32), None, {}
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.sock.settimeout(self.remaining()); self.sock.connect(target)
            self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self.read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'DFHack handshake mismatch')
            for name in METHODS:
                raw = self.frame(0, encode({1: name.encode(), 2: b'dfmcp.order_run.v1_14.Request',
                                           3: b'dfmcp.order_run.v1_14.Reply', 4: b'dfmcp_order_run_v1_14'}))
                bound = decode(raw, 1); require(set(bound) == {1}, 'invalid binding reply')
                method = integer(bound[1], 2, 32767); require(method not in self.methods.values(), 'method bindings alias')
                self.methods[name] = method
            self.call('Handshake')
        except BaseException:
            self.close(); raise

    def remaining(self):
        value = self.deadline - time.monotonic(); require(value >= .001, 'shared native deadline expired'); return value

    def send(self, data):
        self.sock.settimeout(self.remaining()); self.sock.sendall(data)

    def read(self, length):
        out = bytearray()
        while len(out) < length:
            self.sock.settimeout(self.remaining()); part = self.sock.recv(length - len(out))
            require(part, 'connection closed before complete reply'); out += part
        return bytes(out)

    def frame(self, method, data):
        self.send(struct.pack('<h2xi', method, len(data)) + data)
        count, total = 0, 0
        while True:
            kind, width = struct.unpack('<h2xi', self.read(8))
            require(kind in (-1, -3) and 0 <= width <= (4096 if kind == -1 else 65536), 'invalid native frame')
            if kind == -3:
                count += 1; total += width
                require(count <= 8 and total <= 262144, 'native notification budget exceeded')
            payload = self.read(width)
            if kind == -1:
                return payload

    def call(self, operation, operation_key=None, encoded_plan=None, order_id=None):
        require(operation in METHODS, 'operation outside closed profile')
        fields = {1: self.secret, 2: self.nonce, 3: 1, 4: 14}
        if operation == 'ObserveRun':
            fields[11] = integer(order_id, 0, 2**31 - 1)
        elif operation not in ('Handshake', 'ObserveRun'):
            goal = plan(encoded_plan); digest = plan_digest(encoded_plan)
            fields.update({5: key(operation_key).encode(), 9: digest})
            if operation == 'PrepareRun':
                fields.update({6: goal['game_ticks'], 7: goal['wall_ms'], 8: bytes.fromhex(goal['capture_hex']),
                               11: goal['before']['order_id'], 12: goal['predicate'], 13: goal['threshold'],
                               14: goal['samples'], 15: goal['interval']})
            elif operation != 'QueryRun':
                fields[10] = token(operation_key, digest)
        try:
            reply = decode(self.frame(self.methods[operation], encode(fields)))
            require(set(range(1, 9)) <= set(reply) and reply[3] == self.nonce and reply[4] == 1 and reply[5] == 14,
                    'required reply identity mismatch')
            integer(reply[1], 0, 1); integer(reply[2], 0, 8)
            if not reply[1]:
                require(reply[2] != 0 and set(reply) == set(range(1, 9)), 'malformed native refusal')
                raise Rejected('native refusal; no effect outcome inferred')
            require(reply[2] == 0 and {11, 12} <= set(reply), 'missing ownership metadata')
            require(type(reply[7]) is bytes and type(reply[8]) is bytes, 'invalid version wire type')
            current = dict(generation=integer(reply[6], 1, 2**64 - 2),
                           df_version=text(reply[7].decode('utf-8')), dfhack_version=text(reply[8].decode('utf-8')))
            if self.manifest:
                require(current['generation'] >= self.manifest['generation'] and all(current[k] == self.manifest[k]
                        for k in ('df_version', 'dfhack_version')), 'source regressed or software changed')
            self.manifest = current
            active, retained = integer(reply[11], 0, 1), integer(reply[12], 0, 256)
            require(not active or retained > 0, 'active owner has no retained records')
            require((9 in reply) == (operation == 'ObserveRun') and (operation == 'QueryRun'
                    or (10 in reply) == (operation in ('PrepareRun', 'CommitRun', 'CancelRun'))), 'unexpected result fields')
            result = dict(manifest=current, owner_active=bool(active), retained_records=retained)
            if 9 in reply:
                value = capture(reply[9]); require(value['generation'] == current['generation'] and value['order_id'] == order_id,
                                                   'observation from another source or target')
                result.update(capture=value, capture_hex=reply[9].hex())
            if 10 in reply:
                value = record(reply[10]); require(value['key'] == operation_key and value['plan_hex'] == encoded_plan.hex()
                    and value['goal']['before']['generation'] <= current['generation'] and retained > 0, 'receipt from another intent/source')
                if value['phase'] in ('running', 'stopping'):
                    require(active and value['goal']['before']['generation'] == current['generation'], 'running receipt lacks native owner')
                result.update(record=value, record_hex=reply[10].hex())
            return result
        except BaseException:
            self.close(); raise

    def close(self):
        self.sock.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
