"""Closed workforce/1.17 codec and foreground RPC. No ambient mutation authority."""
from __future__ import annotations

import copy
import hashlib
import ipaddress
import re
import secrets
import socket
import struct
import time

METHODS = ('Handshake', 'ObserveWorkforce', 'PrepareAssignment', 'CommitAssignment',
           'QueryAssignment', 'CancelAssignment')
PHASES = ('prepared', 'unknown', 'applied', 'refused', 'cancelled')
MAX_TICK = (2**32 - 1) * 403200 + 403199
MAX_CAPTURE, MAX_EFFECT, MAX_REPLY = 65536, 8192, 131072


class Rejected(ValueError):
    """Bounded refusal; never proof that a failed commit had no effect."""


def require(value, message):
    if not value:
        raise Rejected(message)


def integer(value, lo, hi):
    require(type(value) is int and lo <= value <= hi, 'integer outside profile bounds')
    return value


def boolean(value):
    require(type(value) is bool, 'boolean required')
    return value


def text(value, limit, empty=False):
    require(type(value) is str and (empty or value) and '\0' not in value
            and len(value.encode('utf-8')) <= limit, 'invalid bounded UTF-8 text')
    return value


def key(value):
    require(type(value) is str and re.fullmatch(r'[A-Za-z0-9_.-]{1,128}', value), 'invalid assignment key')
    return value


def ids(value, maximum=32, empty=False):
    require(type(value) is list and (empty or value) and len(value) <= maximum, 'invalid ID collection bound')
    for i, n in enumerate(value):
        integer(n, 0, 2**31-1)
        require(i == 0 or value[i-1] < n, 'IDs must be sorted and unique')
    return value


def unhex(value, lo, hi=None):
    hi = lo if hi is None else hi
    require(type(value) is str and lo*2 <= len(value) <= hi*2 and len(value) % 2 == 0
            and re.fullmatch('[0-9a-f]*', value) is not None, 'invalid canonical hex')
    return bytes.fromhex(value)


def field(raw):
    require(len(raw) <= 65535, 'field length overflow')
    return struct.pack('>H', len(raw)) + raw


def hashed(domain, raw):
    return hashlib.sha256(domain + b'\0' + raw).digest()


class Reader:
    def __init__(self, raw):
        require(type(raw) is bytes, 'wire payload must be bytes')
        self.raw, self.pos = raw, 0

    def take(self, n):
        require(0 <= n <= len(self.raw) - self.pos, 'truncated wire field')
        out = self.raw[self.pos:self.pos+n]; self.pos += n
        return out

    def number(self, n):
        return int.from_bytes(self.take(n), 'big')

    def flag(self):
        return bool(integer(self.number(1), 0, 1))

    def string(self, maximum, empty=False):
        n = self.number(2); require(n <= maximum, 'oversized wire text')
        return text(self.take(n).decode('utf-8'), maximum, empty)

    def bits(self, n):
        raw = self.take(n); require(all(b in (0, 1) for b in raw), 'noncanonical labor mask')
        return list(raw)

    def end(self):
        require(self.pos == len(self.raw), 'trailing wire bytes')


def unit(r, count):
    return {'id': integer(r.number(4), 0, 2**31-1),
            'historical_id': integer(r.number(4), 0, 2**31-1),
            'eligible': r.flag(), 'labors': r.bits(count)}


def capture(raw):
    require(len(raw) <= MAX_CAPTURE, 'capture exceeds 64 KiB')
    r = Reader(raw); require(r.take(8) == b'DFMWF017', 'wrong workforce capture generation')
    out = {'generation': integer(r.number(8), 1, 2**64-2), 'sequence': r.number(8),
           'tick': integer(r.number(8), 0, MAX_TICK), 'site': integer(r.number(4), 0, 2**31-1),
           'folder': r.string(512), 'paused': r.flag(), 'automatic': r.flag()}
    count = integer(r.number(2), 1, 128)
    keys = [r.string(64) for _ in range(count)]
    require(len(set(keys)) == count, 'duplicate native labor keys')
    details, total = [], 0
    for _ in range(integer(r.number(2), 0, 64)):
        d = {'name': r.string(256, True), 'flags': r.number(4), 'selected_only': r.flag(), 'labors': r.bits(count)}
        n = integer(r.number(2), 0, 4096); total += n
        require(total <= 4096, 'aggregate membership bound exceeded')
        d['members'] = ids([r.number(4) for _ in range(n)], 4096, True)
        details.append(d)
    units = [unit(r, count) for _ in range(integer(r.number(2), 1, 32))]
    ids([u['id'] for u in units]); r.end()
    out.update(labor_keys=keys, details=details, units=units)
    return out


def encode_unit(u):
    return struct.pack('>IIB', u['id'], u['historical_id'], u['eligible']) + bytes(u['labors'])


def encode_capture(value):
    """Reconstruct a complete post-witness from decoded values, never raw offsets."""
    v = value
    raw = b'DFMWF017' + struct.pack('>QQQI', v['generation'], v['sequence'], v['tick'], v['site'])
    raw += field(v['folder'].encode()) + bytes([v['paused'], v['automatic']])
    raw += struct.pack('>H', len(v['labor_keys'])) + b''.join(field(k.encode()) for k in v['labor_keys'])
    raw += struct.pack('>H', len(v['details']))
    for d in v['details']:
        raw += field(d['name'].encode()) + struct.pack('>IB', d['flags'], d['selected_only']) + bytes(d['labors'])
        raw += struct.pack('>H', len(d['members'])) + b''.join(struct.pack('>I', n) for n in d['members'])
    raw += struct.pack('>H', len(v['units'])) + b''.join(encode_unit(u) for u in v['units'])
    require(capture(raw) == value, 'noncanonical workforce capture values')
    return raw


def expected(before, detail, assigned):
    integer(detail, 0, 63); boolean(assigned)
    require(before['paused'] and before['automatic'] and before['sequence'] < 2**64-1
            and detail < len(before['details']), 'ineligible workforce precondition')
    selected = before['details'][detail]
    require(selected['selected_only'] and any(selected['labors']) and all(u['eligible'] for u in before['units']),
            'selected-only detail and eligible adult citizens required')
    selected_ids = [u['id'] for u in before['units']]
    changed = [n for n in selected_ids if (n in selected['members']) != assigned]
    require(changed, 'assignment is already in the requested state')
    out = copy.deepcopy(before); out['sequence'] += 1
    members = set(selected['members'])
    members = members.union(selected_ids) if assigned else members.difference(selected_ids)
    out['details'][detail]['members'] = sorted(members)
    encode_capture(out)  # Enforce capacity of the complete expected post-configuration.
    return out, changed


PLAN_FIELDS = {'key', 'detail_index', 'assigned', 'capture_hex', 'plan_digest', 'prepare_token'}


def make_plan(operation_key, detail, assigned, raw):
    key(operation_key); before = capture(raw); expected(before, detail, assigned)
    spec = struct.pack('>IB', detail, assigned)
    digest = hashed(b'dfmcp-workforce-plan/1', spec + hashlib.sha256(raw).digest())
    token = hashed(b'dfmcp-workforce-token/1', field(operation_key.encode()) + digest)[:16]
    return {'key': operation_key, 'detail_index': detail, 'assigned': assigned, 'capture_hex': raw.hex(),
            'plan_digest': digest.hex(), 'prepare_token': token.hex()}


def validate_plan(value):
    require(type(value) is dict and set(value) == PLAN_FIELDS, 'unexpected plan fields')
    result = make_plan(value['key'], value['detail_index'], value['assigned'], unhex(value['capture_hex'], 1, MAX_CAPTURE))
    require(result == value, 'plan commitment mismatch')
    return value


def effect(raw, plan):
    validate_plan(plan); before_raw = bytes.fromhex(plan['capture_hex']); before = capture(before_raw)
    require(len(raw) <= MAX_EFFECT, 'effect exceeds 8 KiB')
    r = Reader(raw); require(r.take(8) == b'DFMWE017', 'wrong workforce effect generation')
    require(r.string(128) == plan['key'] and r.take(32).hex() == plan['plan_digest']
            and r.take(16).hex() == plan['prepare_token'], 'effect identity mismatch')
    require(r.number(4) == plan['detail_index'] and r.flag() == plan['assigned'], 'effect spec mismatch')
    require(r.take(32) == hashlib.sha256(before_raw).digest(), 'effect witness mismatch')
    require([r.number(8) for _ in range(3)] == [before[k] for k in ('generation', 'sequence', 'tick')], 'effect source mismatch')
    phase = PHASES[integer(r.number(1), 0, 4)]; after_hash = r.take(32)
    columns = integer(r.number(2), 1, 128); require(columns == len(before['labor_keys']), 'effect labor columns changed')
    post = [unit(r, columns) for _ in range(integer(r.number(2), 0, 32))]
    checksum = r.take(32); r.end()
    require(checksum == hashed(b'dfmcp-workforce-receipt/1', raw[:-32]), 'effect checksum mismatch')
    changed = []
    if phase == 'applied':
        after, changed = expected(before, plan['detail_index'], plan['assigned'])
        require(len(post) == len(before['units']), 'missing selected post-state')
        for i, u in enumerate(post):
            old = before['units'][i]
            require(all(u[k] == old[k] for k in ('id', 'historical_id', 'eligible')), 'selected identity/eligibility drift')
            if u['id'] in changed:
                if plan['assigned']:
                    allowed = before['details'][plan['detail_index']]['labors']
                    require(all(not bit or u['labors'][j] for j, bit in enumerate(allowed)), 'assignment did not enable allowed labors')
                after['units'][i]['labors'] = u['labors']
            else:
                require(u == old, 'unchanged citizen recomputed or modified')
        require(after_hash == hashlib.sha256(encode_capture(after)).digest(), 'post-configuration witness mismatch')
    else:
        require(not post and after_hash == bytes(32), 'non-applied record carries invented readback')
    return {'phase': phase, 'receipt_digest': checksum.hex(), 'after_witness': after_hash.hex() if post else None,
            'post_units': post, 'changed_ids': changed, 'current_state_proven': False, 'job_completion_proven': False}


def address(raw):
    require(type(raw) is str and re.fullmatch(r'[0-9.]+:[0-9]{1,5}', raw), 'numeric IPv4 loopback address required')
    host, port = raw.split(':'); ip = ipaddress.IPv4Address(host); port = integer(int(port), 1, 65535)
    require(ip.is_loopback and raw == f'{ip}:{port}', 'canonical loopback endpoint required')
    return str(ip), port


def varint(n):
    integer(n, 0, 2**64-1); out = bytearray()
    while n >= 128:
        out.append(n % 128 + 128); n //= 128
    return bytes(out + bytes([n]))


def encode(fields):
    """Pairs preserve the schema's explicit unpacked repeated unit-ID field."""
    out = b''
    for n, v in fields:
        integer(n, 1, 12)
        out += (varint(n*8+2) + varint(len(v)) + v) if type(v) is bytes else (varint(n*8) + varint(v))
    require(len(out) <= 2048, 'native request exceeds 2 KiB')
    return out


def decode(raw, maximum=12):
    require(len(raw) <= MAX_REPLY, 'native reply exceeds its bound'); r = Reader(raw); out = {}
    def number():
        n = 0
        for i in range(10):
            b = r.number(1); require(i < 9 or b <= 1, 'varint overflow'); n |= (b & 127) << (7*i)
            if b < 128:
                require(i == 0 or b != 0, 'overlong varint'); return n
        raise Rejected('unterminated varint')
    while r.pos < len(raw):
        tag = number(); n, kind = tag >> 3, tag & 7
        require(1 <= n <= maximum and n not in out and kind in (0, 2), 'unknown/duplicate protobuf field')
        out[n] = number() if kind == 0 else r.take(number())
    return out


class Client:
    """One foreground socket with one absolute deadline; never reconnect/replay."""
    def __init__(self, endpoint, secret, deadline):
        self.endpoint, self.deadline = endpoint, deadline
        target = address(endpoint); require(type(secret) is bytes and 32 <= len(secret) <= 256, 'invalid credential length')
        self.secret, self.nonce, self.manifest, self.methods = secret, secrets.token_bytes(32), None, {}
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.sock.settimeout(self.remaining()); self.sock.connect(target)
            self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.send(b'DFHack?\n' + struct.pack('<i', 1))
            require(self.read(12) == b'DFHack!\n' + struct.pack('<i', 1), 'native handshake mismatch')
            for name in METHODS:
                bound = decode(self.frame(0, encode([(1,name.encode()), (2,b'dfmcp.workforce.v1_17.Request'),
                    (3,b'dfmcp.workforce.v1_17.Reply'), (4,b'dfmcp_workforce_v1_17')])), 1)
                require(set(bound) == {1}, 'invalid method binding')
                method = integer(bound[1], 2, 32767)
                require(method not in self.methods.values(), 'aliased native bindings'); self.methods[name] = method
            self.call('Handshake')
        except BaseException:
            self.close(); raise

    def remaining(self):
        value = self.deadline - time.monotonic(); require(value >= 0.001, 'shared RPC deadline exhausted')
        return value

    def send(self, data):
        self.sock.settimeout(self.remaining()); self.sock.sendall(data)

    def read(self, n):
        out = bytearray()
        while len(out) < n:
            self.sock.settimeout(self.remaining()); part = self.sock.recv(n-len(out))
            require(part, 'connection closed before complete reply'); out += part
        return bytes(out)

    def frame(self, method, request):
        self.send(struct.pack('<h2xi', method, len(request)) + request); count, total = 0, 0
        while True:
            tag, n = struct.unpack('<h2xi', self.read(8))
            require(tag in (-1, -3), 'native failure or unknown response frame')
            require(0 <= n <= (MAX_REPLY if tag == -1 else 65536), 'frame size refused')
            if tag == -3:
                count += 1; total += n; require(count <= 8 and total <= 262144, 'notification budget exhausted')
            value = self.read(n)
            if tag == -1: return value

    def call(self, operation, plan=None, unit_ids=None):
        require(operation in METHODS, 'operation outside workforce profile')
        fields = [(1,self.secret), (2,self.nonce), (3,1), (4,17)]
        if operation == 'ObserveWorkforce':
            fields += [(11,n) for n in ids(unit_ids)]
        elif operation != 'Handshake':
            validate_plan(plan)
            fields += [(5,plan['key'].encode()), (9,bytes.fromhex(plan['plan_digest']))]
            if operation == 'PrepareAssignment':
                before = bytes.fromhex(plan['capture_hex'])
                fields += [(6,plan['detail_index']), (7,int(plan['assigned'])), (8,hashlib.sha256(before).digest())]
                fields += [(11,u['id']) for u in capture(before)['units']]
            elif operation in ('CommitAssignment','CancelAssignment'):
                fields += [(10,bytes.fromhex(plan['prepare_token']))]
        try:
            reply = decode(self.frame(self.methods[operation], encode(fields)))
            require(set(range(1,9)) <= set(reply), 'required reply fields missing')
            require(reply[3] == self.nonce and reply[4] == 1 and reply[5] == 17, 'reply nonce/profile mismatch')
            accepted = integer(reply[1],0,1); code = integer(reply[2],0,8)
            if not accepted:
                require(code != 0 and set(reply) == set(range(1,9)), 'malformed refusal')
                raise Rejected(f'native refusal {code}; no effect outcome inferred')
            require(code == 0 and {11,12} <= set(reply), 'success lacks coordination metadata')
            require(type(reply[7]) is bytes and type(reply[8]) is bytes, 'invalid software version wire types')
            current = {'generation':integer(reply[6],1,2**64-2),
                       'df_version':text(reply[7].decode('utf-8'),128), 'dfhack_version':text(reply[8].decode('utf-8'),128)}
            if self.manifest:
                require(current['generation'] >= self.manifest['generation'] and all(current[k] == self.manifest[k]
                        for k in ('df_version','dfhack_version')), 'native source regressed or software changed')
            self.manifest = current
            result = {'manifest':current, 'unresolved':bool(integer(reply[11],0,1)), 'retained_records':integer(reply[12],0,64)}
            require((9 in reply) == (operation == 'ObserveWorkforce'), 'unexpected/missing capture')
            require(operation == 'QueryAssignment' or (10 in reply) == (operation in
                    ('PrepareAssignment','CommitAssignment','CancelAssignment')), 'unexpected/missing effect')
            if 9 in reply:
                observed = capture(reply[9]); require(observed['generation'] == current['generation']
                    and [u['id'] for u in observed['units']] == unit_ids, 'wrong source/selected units')
                result.update(capture_hex=reply[9].hex())
            if 10 in reply:
                observed = effect(reply[10],plan)
                require(capture(bytes.fromhex(plan['capture_hex']))['generation'] == current['generation'], 'effect from another incarnation')
                require(result['retained_records'] > 0 and (observed['phase'] != 'unknown' or result['unresolved']), 'unowned effect evidence')
                result.update(effect_hex=reply[10].hex())
            return result
        except BaseException:
            self.close(); raise

    def close(self): self.sock.close()
    def __enter__(self): return self
    def __exit__(self, *_): self.close()
