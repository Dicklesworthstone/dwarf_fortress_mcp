#!/usr/bin/env python3
"""Combined modern stdio component probe. Peers are doubles, never a game."""
from __future__ import annotations
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import socket
import struct
import sys
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import workforce_wire as wf
import order_run_wire as run_wire
import bounded_run_client as inventory_run_wire
spec = importlib.util.spec_from_file_location('workshop_stdio', ROOT / 'tests/stdio/workshop.py')
stdio = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stdio)
number, blob, fields, receive = stdio.number, stdio.blob, stdio.fields, stdio.receive

DF = b'0.51.11'
DFH = b'0.51.11-r1'
GEN = 701
WGEN = 7
OGEN = 53
CLOCK_GENERATION = 77
INVENTORY_CLOCK_GENERATION = 78
PGEN = 70
IGEN = 71
TICK = 120
SITE = 4
TOKEN = b't' * 32


def text(value):
    return len(value).to_bytes(2, 'big') + value


def workforce_capture(ids=(42,), option_count=0):
    keys = [f'LABOR_{i}' for i in range(option_count)] if option_count else ['MINE', 'CARPENTRY']
    details = [{'name': f'Only {key}', 'flags': 0, 'selected_only': True,
                'labors': [int(i == column) for i in range(len(keys))], 'members': []}
               for column, key in enumerate(keys)] if option_count else [
                   {'name': 'Mining only', 'flags': 0, 'selected_only': True,
                    'labors': [1, 0], 'members': []}]
    return wf.encode_capture({'generation': WGEN, 'sequence': 3, 'tick': TICK, 'site': SITE,
        'folder': 'region1', 'paused': True, 'automatic': True, 'labor_keys': keys,
        'details': details,
        'units': [{'id': ident, 'historical_id': ident, 'eligible': True,
                   'labors': [0] * len(keys)} for ident in ids]})


def workforce_fields(raw):
    """Decode only the documented unpacked repeated unit selector specially."""
    at = 0
    out = {}
    def take():
        nonlocal at
        value = shift = 0
        while True:
            byte = raw[at]
            at += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
            assert shift < 70
    while at < len(raw):
        key = take()
        tag, kind = key >> 3, key & 7
        if kind == 0:
            value = take()
        else:
            assert kind == 2
            size = take()
            value = raw[at:at + size]
            assert len(value) == size
            at += size
        if tag == 11:
            assert kind == 0
            out.setdefault(tag, []).append(value)
        else:
            assert tag not in out
            out[tag] = value
    return out


def workforce_effect(plan, applied):
    before = wf.capture(bytes.fromhex(plan['capture_hex']))
    raw = b'DFMWE017' + text(plan['key'].encode()) + bytes.fromhex(plan['plan_digest']) + bytes.fromhex(plan['prepare_token'])
    raw += struct.pack('>IB', plan['detail_index'], plan['assigned']) + hashlib.sha256(bytes.fromhex(plan['capture_hex'])).digest()
    raw += struct.pack('>QQQ', before['generation'], before['sequence'], before['tick'])
    if applied:
        after, changed = wf.expected(before, plan['detail_index'], plan['assigned'])
        assert changed == [unit['id'] for unit in before['units']]
        for unit in after['units']:
            for column, enabled in enumerate(before['details'][plan['detail_index']]['labors']):
                if enabled:
                    unit['labors'][column] = int(plan['assigned'])
        raw += b'\x02' + hashlib.sha256(wf.encode_capture(after)).digest()
        raw += struct.pack('>HH', len(before['labor_keys']), len(after['units']))
        raw += b''.join(wf.encode_unit(unit) for unit in after['units'])
    else:
        raw += b'\x00' + bytes(32) + struct.pack('>HH', len(before['labor_keys']), 0)
    return raw + wf.hashed(b'dfmcp-workforce-receipt/1', raw)


def order_queue(created=False, *, tick=TICK, sequence=None, retired=False):
    present = created and not retired
    return (b'DFMWO010' + struct.pack('>QQQII', OGEN, 11 + created if sequence is None else sequence,
                                   tick, 100 + created, SITE)
            + b'\x01' + text(b'region1') + struct.pack('>I', int(present))
            + (struct.pack('>I', 100) if present else b''))


def progress_capture(tick, sequence, present=True, amount=1, remaining=None, status=3):
    """Independent progress/1.12 bytes; absent orders retain their allocation horizon."""
    raw = (b'DFMWP012' + struct.pack('>QQQIIIB', PGEN, sequence, tick, SITE, 101, int(present), 1)
           + text(b'region1') + struct.pack('>IIB', 1, 100, int(present)))
    if present:
        raw += (struct.pack('>iBiiIiiiiiII', 69, 1, amount if remaining is None else remaining,
                            amount, status, 0, -1, 1, -1, -1, 0, 0)
                + text(b'ConstructBed') + text(b''))
    return raw


def clock_capture(sequence, tick, retired=False):
    return (b'DFMOR014' + struct.pack('>QQQI', CLOCK_GENERATION, sequence, tick, SITE)
            + text(b'region1') + struct.pack('>BIIBBiiI', 1, 100, 101, int(not retired),
                                           0 if retired else 1, 0 if retired else 1,
                                           0 if retired else 1, 0 if retired else 3))


def clock_record(encoded_plan, key, phase=0, reason=0, observed=None):
    before = run_wire.plan(encoded_plan)['before']
    digest = hashlib.sha256(b'dfmcp-order-run-plan/1\0' + encoded_plan).digest()
    token = hashlib.sha256(b'dfmcp-order-run-token/1\0' + text(key) + digest).digest()[:16]
    payload = (b'DFMOE014' + text(key) + text(encoded_plan) + digest + token
               + bytes([phase, reason, 0, int(phase in (1, 2, 3, 5)), int(phase == 3),
                        int(observed is not None)])
               + struct.pack('>QIQH', observed or 0, 0, before['tick'], 0))
    result = payload + hashlib.sha256(b'dfmcp-order-run-receipt/1\0' + payload).digest()
    run_wire.record(result)
    return result


def inventory_clock_capture(sequence, tick, paused=True):
    return b'DFMRO013' + struct.pack('>QQQBBB', INVENTORY_CLOCK_GENERATION, sequence, tick,
                                    1, 1, int(paused))


def inventory_clock_record(plan, phase=0, reason=0, observed=None):
    payload = (b'DFMRE013' + text(plan['key']) + struct.pack('>II', plan['ticks'], plan['wall'])
               + plan['before'] + plan['digest'] + plan['token']
               + bytes([phase, reason, int(phase in (1, 2, 3, 5)), int(phase == 3),
                        int(observed is not None)]) + struct.pack('>Q', observed or 0))
    result = payload + hashlib.sha256(b'dfmcp-bounded-run-receipt/1\0' + payload).digest()
    inventory_run_wire.decode_record(result)
    return result


def operations_capture(tick, stack=1):
    """One complete operations/1.3 inventory, with no job/output attribution."""
    year, annual = divmod(tick, 403200)
    jobs = (b'DFMJ1200' + struct.pack('>IIBiI', year, annual, 1, SITE, 100)
            + text(b'region1') + struct.pack('>I', 0))
    raw = b'DFMO1300' + struct.pack('>I', len(jobs)) + jobs + struct.pack('>III', 100, 100, 0)
    raw += struct.pack('>I', int(stack > 0))
    if stack > 0:
        raw += (struct.pack('>Ii', 2, 5) + text(b'BED') + struct.pack('>iiiIiiiI', -1, 420, 0, stack, 1, 1, 1, 64)
                + b'\x00\x00')
    return raw + struct.pack('>I', 0)


def order_effect(plan, created):
    before = order_queue(False)
    key, recipe, amount = plan['key'], plan['recipe'], plan['amount']
    spec = bytes([recipe]) + struct.pack('>I', amount)
    digest = hashlib.sha256(b'dfmcp-work-order-plan/1\0' + spec + hashlib.sha256(before).digest()).digest()
    token = hashlib.sha256(b'dfmcp-work-order-token/1\0' + struct.pack('>Q', OGEN) + text(key) + digest).digest()[:16]
    assert digest == plan['digest']
    raw = b'DFMWOE10' + struct.pack('>QQQI', OGEN, 11, TICK, 100) + spec
    raw += hashlib.sha256(before).digest() + digest + token
    raw += bytes([2 if created else 0, int(created)]) + struct.pack('>Q', TICK if created else 0)
    raw += hashlib.sha256(order_queue(True)).digest() if created else bytes(32)
    raw += hashlib.sha256(b'DFMWOC10' + struct.pack('>I', 100) + spec).digest() if created else bytes(32)
    proof = (hashlib.sha256(b'dfmcp-work-order-receipt/1\0' + struct.pack('>Q', OGEN) + text(key) + raw[73:195]).digest()
             if created else bytes(32))
    return raw + proof + text(key)


def citizen_reply(op, req, citizen_ids=(42,)):
    if op == 'Handshake':
        assert req[6] == TOKEN
        return (number(1, 1) + blob(2, b'') + blob(3, b'') + number(4, 1) + number(5, 0)
                + blob(6, b'0.1.0') + blob(7, DFH) + blob(8, DF) + number(9, 1) + number(10, 1)
                + blob(11, req[5]) + number(12, GEN) + blob(13, b'Handshake') + blob(13, b'ReadObservation'))
    assert req[4] == TOKEN and 0 <= req[5] <= len(citizen_ids) and req[6] >= 1
    page = citizen_ids[req[5]:req[5] + req[6]]
    citizens = b''
    for ident in page:
        citizen = number(1, ident * 2) + blob(2, f'Urist {ident}'.encode() if req[7] else b'') + blob(3, b'dwarf')
        citizen += number(4, 8) + number(5, 20) + number(6, 22) + number(7, 4)
        citizen += b''.join(number(n, int(n not in (13, 14, 15))) for n in range(8, 17))
        citizens += blob(19, citizen)
    return (number(1, 1) + blob(2, b'') + blob(3, b'') + number(4, 1) + number(5, 0)
            + blob(6, req[3]) + number(7, GEN) + number(8, 1) + number(9, 1) + number(10, 1)
            + number(11, 0) + number(12, TICK) + blob(13, b'Routing fixture') + blob(14, b'region1')
            + number(15, SITE * 2) + number(16, len(citizen_ids)) + number(17, req[5])
            + number(18, int(req[5] + len(page) == len(citizen_ids))) + citizens)


class Peer:
    def __init__(self, directory):
        self.directory = directory
        self.log_lock = threading.Lock()
        self.socket = socket.socket()
        self.socket.bind(('127.0.0.1', 0))
        self.socket.listen(8)
        self.socket.settimeout(.1)
        self.endpoint = '127.0.0.1:' + str(self.socket.getsockname()[1])
        self.stop = threading.Event()
        self.threads = []
        self.errors = []
        self.calls = []
        self.citizen_ids = [42]
        self.option_count = 0
        self.workforce_selections = []
        self.workforce_plan = None
        self.order_plan = None
        self.workforce_applied = False
        self.workforce_generation = WGEN
        self.workforce_sequence = None
        self.workforce_tick = TICK
        self.workforce_labor = None
        self.after_workforce_read = None
        self.order_created = False
        self.order_retired = False
        self.order_remaining = None
        self.order_status = 3
        self.order_tick = TICK
        self.order_sequence = 11
        self.progress_sequence = 0
        self.inventory_stack = 1
        self.after_operations_read = None
        self.drop_workforce_reply = False
        self.clock_sequence = 0
        self.clock_plans = {}
        self.clock_records = {}
        self.clock_commits = []
        self.hold_clock = False
        self.drop_clock_reply = False
        self.inventory_clock_sequence = 0
        self.inventory_clock_plans = {}
        self.inventory_clock_records = {}
        self.inventory_clock_commits = []
        self.hold_inventory_clock = False
        self.drop_inventory_clock_reply = False
        self.after_inventory_clock_observe = None
        self.acceptor = threading.Thread(target=self.accept, name='combined-peer')
        self.acceptor.start()

    def accept(self):
        try:
            while not self.stop.is_set():
                try: stream, _ = self.socket.accept()
                except socket.timeout: continue
                thread = threading.Thread(target=self.connection, args=(stream,))
                self.threads.append(thread); thread.start()
        finally:
            self.socket.close()

    def connection(self, stream):
        try:
            with stream:
                stream.settimeout(10)
                assert receive(stream, 12) == b'DFHack?\n\x01\0\0\0'
                stream.sendall(b'DFHack!\n\x01\0\0\0')
                bound = {}
                while not self.stop.is_set():
                    try: method, pad, size = struct.unpack('<hhI', receive(stream, 8))
                    except EOFError: break
                    assert pad == 0 and 0 <= size <= 131072
                    wire_request = receive(stream, size)
                    req = (workforce_fields(wire_request)
                           if method and bound[method][0] == 'dfmcp_workforce_v1_17'
                           else fields(wire_request))
                    if method == 0:
                        op, plugin = req[1].decode(), req[4].decode()
                        ident = len(bound) + 2
                        bound[ident] = (plugin, op)
                        response = number(1, ident)
                    else:
                        plugin, op = bound[method]
                        self.calls.append((plugin, op))
                        with self.log_lock, (self.directory / 'native-requests.jsonl').open('a', encoding='utf-8') as log:
                            log.write(json.dumps({'plugin': plugin, 'method': op, 'request_hex': wire_request.hex()}) + '\n')
                        response = self.reply(plugin, op, req)
                        with self.log_lock, (self.directory / 'native-replies.jsonl').open('a', encoding='utf-8') as log:
                            log.write(json.dumps({'plugin': plugin, 'method': op, 'reply_hex': response.hex()}) + '\n')
                    stream.sendall(struct.pack('<hhI', -1, 0, len(response)) + response)
        except (EOFError, ConnectionResetError, BrokenPipeError):
            pass
        except BaseException as error:
            self.errors.append(repr(error))

    def reply(self, plugin, op, req):
        if plugin == 'dfmcp_bridge': return citizen_reply(op, req, self.citizen_ids)
        assert req[1] == TOKEN
        if plugin == 'dfmcp_run_v1_13':
            assert req[3] == 1 and req[4] == 13
            response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 13)
                        + number(6, INVENTORY_CLOCK_GENERATION) + blob(7, DF) + blob(8, DFH))
            if op == 'ObserveRun':
                response += blob(9, inventory_clock_capture(self.inventory_clock_sequence, self.order_tick,
                                                            not self.inventory_clock_active()))
                if self.after_inventory_clock_observe is not None:
                    hook, self.after_inventory_clock_observe = self.after_inventory_clock_observe, None
                    hook()
            elif op == 'PrepareRun':
                assert req[8] == inventory_clock_capture(self.inventory_clock_sequence, self.order_tick)
                digest = inventory_run_wire.plan_for(req[6], req[7], req[8])
                assert req[9] == digest
                assert req[5] not in self.inventory_clock_plans
                plan = {'key': req[5], 'ticks': req[6], 'wall': req[7], 'before': req[8],
                        'digest': digest, 'token': inventory_run_wire.token_for(req[5].decode(), digest)}
                self.inventory_clock_plans[req[5]] = plan
                self.inventory_clock_records[req[5]] = inventory_clock_record(plan)
                response += blob(10, self.inventory_clock_records[req[5]])
            elif op != 'Handshake':
                assert op in ('CommitRun', 'QueryRun', 'CancelRun')
                key = req[5]
                if key in self.inventory_clock_plans:
                    plan = self.inventory_clock_plans[key]
                    assert req[9] == plan['digest']
                    if op != 'QueryRun':
                        assert req[10] == plan['token']
                    if op == 'CommitRun':
                        assert key not in self.inventory_clock_commits, 'native unpause must never be retried'
                        self.inventory_clock_commits.append(key)
                        if self.hold_inventory_clock:
                            self.inventory_clock_records[key] = inventory_clock_record(plan, phase=1,
                                                                                       observed=self.order_tick)
                        else:
                            self.finish_inventory_clock(key)
                        if self.drop_inventory_clock_reply:
                            self.drop_inventory_clock_reply = False
                            raise ConnectionResetError('lost inventory clock reply after retained unpause')
                    elif op == 'CancelRun':
                        if key in self.inventory_clock_commits:
                            self.inventory_clock_records[key] = inventory_clock_record(plan, phase=3, reason=3,
                                                                                       observed=self.order_tick)
                        else:
                            self.inventory_clock_records[key] = inventory_clock_record(plan, phase=4, reason=3)
                    response += blob(10, self.inventory_clock_records[key])
            return (response + number(11, int(self.inventory_clock_active()))
                    + number(12, len(self.inventory_clock_records)))
        if plugin == 'dfmcp_order_run_v1_14':
            assert req[3] == 1 and req[4] == 14
            response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 14)
                        + number(6, CLOCK_GENERATION) + blob(7, DF) + blob(8, DFH))
            if op == 'ObserveRun':
                assert req[11] == 100
                response += blob(9, clock_capture(self.clock_sequence, self.order_tick, self.order_retired))
            elif op == 'PrepareRun':
                assert req[8] == clock_capture(self.clock_sequence, self.order_tick, self.order_retired)
                assert req[11] == 100
                plan = (b'DFMOP014' + struct.pack('>IIBIII', req[6], req[7], req[12], req[13],
                                                req[14], req[15]) + text(req[8]))
                assert req[9] == run_wire.plan_digest(plan)
                assert req[5] not in self.clock_plans
                self.clock_plans[req[5]] = plan
                self.clock_records[req[5]] = clock_record(plan, req[5])
                response += blob(10, self.clock_records[req[5]])
            elif op != 'Handshake':
                assert op in ('CommitRun', 'QueryRun', 'CancelRun')
                key = req[5]
                if key in self.clock_plans:
                    plan = self.clock_plans[key]
                    assert req[9] == run_wire.plan_digest(plan)
                    if op != 'QueryRun':
                        assert req[10] == run_wire.token(key.decode(), req[9])
                    if op == 'CommitRun':
                        assert key not in self.clock_commits, 'native unpause must never be retried'
                        self.clock_commits.append(key)
                        if self.hold_clock:
                            self.clock_records[key] = clock_record(plan, key, phase=1, observed=self.order_tick)
                        else:
                            self.finish_clock(key)
                        if self.drop_clock_reply:
                            self.drop_clock_reply = False
                            raise ConnectionResetError('lost clock reply after retained unpause')
                    elif op == 'CancelRun':
                        if key in self.clock_commits:
                            self.clock_records[key] = clock_record(plan, key, phase=3, reason=3,
                                                                   observed=self.order_tick)
                        else:
                            self.clock_records[key] = clock_record(plan, key, phase=4, reason=3)
                    response += blob(10, self.clock_records[key])
            active = any(run_wire.record(raw)['phase'] in ('running', 'stopping')
                         for raw in self.clock_records.values())
            return response + number(11, int(active)) + number(12, len(self.clock_records))
        if plugin == 'dfmcp_workforce_v1_17':
            response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 17)
                        + number(6, self.workforce_generation) + blob(7, DF) + blob(8, DFH))
            if op == 'ObserveWorkforce':
                ids = req[11]
                assert ids == sorted(set(ids)) and 1 <= len(ids) <= 32
                assert all(ident in self.citizen_ids for ident in ids)
                self.workforce_selections.append(ids)
                capture = wf.capture(workforce_capture(ids, self.option_count))
                if self.workforce_applied:
                    before = wf.capture(bytes.fromhex(self.workforce_plan['capture_hex']))
                    assert [unit['id'] for unit in before['units']] == ids
                    capture, _ = wf.expected(before, self.workforce_plan['detail_index'],
                                             self.workforce_plan['assigned'])
                    enabled = (self.workforce_plan['assigned'] if self.workforce_labor is None
                               else self.workforce_labor)
                    detail = capture['details'][self.workforce_plan['detail_index']]
                    detail['members'] = ids if enabled else []
                    for unit in capture['units']:
                        for column, assigned in enumerate(detail['labors']):
                            if assigned:
                                unit['labors'][column] = int(enabled)
                capture['generation'] = self.workforce_generation
                if self.workforce_sequence is not None:
                    capture['sequence'] = self.workforce_sequence
                capture['tick'] = self.workforce_tick
                response += blob(9, wf.encode_capture(capture))
                if self.after_workforce_read is not None:
                    self.after_workforce_read()
            if op == 'PrepareAssignment':
                capture = workforce_capture(req[11], self.option_count)
                self.workforce_plan = wf.make_plan(req[5].decode(), req[6], bool(req[7]), capture)
                assert bytes.fromhex(self.workforce_plan['plan_digest']) == req[9]
                assert hashlib.sha256(capture).digest() == req[8]
                response += blob(10, workforce_effect(self.workforce_plan, False))
            if op == 'CommitAssignment':
                assert self.workforce_plan and req[5].decode() == self.workforce_plan['key']
                assert req[10] == bytes.fromhex(self.workforce_plan['prepare_token'])
                assert not self.workforce_applied
                self.workforce_applied = True
                if self.drop_workforce_reply:
                    raise ConnectionResetError("injected lost commit reply after retained effect")
                response += blob(10, workforce_effect(self.workforce_plan, True))
            if op == 'QueryAssignment' and self.workforce_plan:
                response += blob(10, workforce_effect(self.workforce_plan, self.workforce_applied))
            return response + number(11, 0) + number(12, int(self.workforce_plan is not None))
        if plugin == 'dfmcp_work_order_progress_v1_12':
            assert req[3] == 1 and req[4] == 12
            assert op in ('Handshake', 'ReadObservation')
            response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 12)
                        + number(6, PGEN) + blob(7, DF) + blob(8, DFH))
            if op == 'ReadObservation':
                assert req[5] == 100 and self.order_created
                self.progress_sequence += 1
                response += blob(9, progress_capture(self.order_tick, self.progress_sequence,
                    not self.order_retired, self.order_plan['amount'],
                    self.order_remaining, self.order_status))
            return response
        if plugin == 'dfmcp_operations_v1_3':
            assert req[3] == 1 and req[4] == 3
            assert {key: req[key] for key in (5, 6, 7, 8)} == {
                5: 1024, 6: 1024, 7: 2046, 8: 2 * 1024 * 1024}
            assert op in ('Handshake', 'ReadObservation')
            response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 3)
                        + number(6, IGEN) + blob(7, DF) + blob(8, DFH))
            if op == 'ReadObservation':
                assert self.order_created
                response += blob(9, operations_capture(self.order_tick, self.inventory_stack))
                if self.after_operations_read is not None:
                    hook, self.after_operations_read = self.after_operations_read, None
                    hook()
            return response
        assert plugin == 'dfmcp_work_orders_v1_10'
        response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 10)
                    + number(6, OGEN) + blob(7, DF) + blob(8, DFH))
        if op == 'ReadOrders':
            response += blob(9, order_queue(self.order_created, tick=self.order_tick,
                                           sequence=self.order_sequence, retired=self.order_retired))
        if op == 'PrepareOrder':
            self.order_plan = {'key': req[5], 'recipe': req[6], 'amount': req[7], 'digest': req[9]}
            assert req[8] == hashlib.sha256(order_queue(False)).digest()
            response += blob(10, order_effect(self.order_plan, False)) + number(11, 0)
        if op == 'CommitOrder':
            assert self.order_plan and req[5] == self.order_plan['key'] and req[9] == self.order_plan['digest']
            token = hashlib.sha256(b'dfmcp-work-order-token/1\0' + struct.pack('>Q', OGEN) + text(req[5]) + req[9]).digest()[:16]
            assert req[10] == token
            assert not self.order_created
            self.order_created = True
            self.order_sequence += 1
            response += blob(10, order_effect(self.order_plan, True))
        if op == 'QueryOrder' and self.order_plan:
            response += blob(10, order_effect(self.order_plan, self.order_created))
        return response

    def finish_clock(self, key):
        plan = self.clock_plans[key]
        parsed = run_wire.plan(plan)
        self.goal_tick(parsed['before']['tick'] + parsed['game_ticks'])
        self.clock_sequence += 1
        self.inventory_stack = 1
        self.clock_records[key] = clock_record(plan, key, phase=3, reason=1, observed=self.order_tick)

    def inventory_clock_active(self):
        return any(inventory_run_wire.decode_record(raw)['phase'] in ('running', 'stopping')
                   for raw in self.inventory_clock_records.values())

    def finish_inventory_clock(self, key):
        plan = self.inventory_clock_plans[key]
        before = inventory_run_wire.snapshot(plan['before'])
        self.goal_tick(before['tick'] + plan['ticks'])
        self.inventory_clock_sequence += 1
        self.inventory_clock_records[key] = inventory_clock_record(plan, phase=3, reason=1,
                                                                   observed=self.order_tick)

    def goal_tick(self, tick):
        """Advance the double explicitly; no native clock or real game is executed."""
        assert self.order_created and self.workforce_applied and tick >= self.order_tick
        if tick > self.order_tick:
            self.order_sequence += 1
        self.order_tick = self.workforce_tick = tick

    def close(self):
        self.stop.set(); self.acceptor.join(3)
        for thread in self.threads: thread.join(12)
        assert not self.acceptor.is_alive() and all(not thread.is_alive() for thread in self.threads)
        assert not self.errors, self.errors


def custody_files(directory):
    result = {}
    for group in ('parent', 'workforce', 'orders'):
        for path in sorted((directory / group).iterdir()):
            assert path.is_file() and not path.is_symlink()
            data = path.read_bytes()
            result[str(path.relative_to(directory))] = (len(data), path.stat().st_mode & 0o7777, hashlib.sha256(data).hexdigest())
    return result


def reopen_environment(env, recovery, workforce_id):
    env = env.copy()
    parent = json.loads(env['DFMCP_COMBINED_PRODUCTION_CONFIG'])
    parent.update(initialize=False, recovery=recovery)
    if parent.get('clock') is not None:
        parent['clock']['journal_id'] = recovery['clock_journal_id']
    if parent.get('inventory_clock') is not None:
        parent['inventory_clock']['journal_id'] = recovery['inventory_clock_journal_id']
    env['DFMCP_COMBINED_PRODUCTION_CONFIG'] = json.dumps(parent)
    for name in ('DFMCP_SEMANTIC_WORKFORCE_CONFIG', 'DFMCP_SEMANTIC_WORK_ORDER_CONFIG'):
        config = json.loads(env[name]); config['initialize'] = False
        if name == 'DFMCP_SEMANTIC_WORKFORCE_CONFIG': config['journal_id'] = workforce_id
        if name == 'DFMCP_SEMANTIC_WORK_ORDER_CONFIG' and config.get('clock') is not None:
            config['clock']['journal_id'] = recovery['clock_journal_id']
        if name == 'DFMCP_SEMANTIC_WORK_ORDER_CONFIG' and config.get('inventory_clock') is not None:
            config['inventory_clock']['journal_id'] = recovery['inventory_clock_journal_id']
        env[name] = json.dumps(config)
    return env


def environment(directory, endpoint):
    paths = {name: directory / name for name in ('parent', 'workforce', 'orders')}
    for path in paths.values(): path.mkdir(mode=0o700)
    common = {'kind': 'operator-development-source-acceptance/1', 'world_folder': 'region1', 'site_id': SITE,
              'df_version': DF.decode(), 'dfhack_version': DFH.decode(), 'citizen_bridge_version': '0.1.0',
              'citizen_generation': GEN}
    workforce = {'schema': 'dfmcp.semantic-workforce-development/1', 'directory': str(paths['workforce']),
                 'endpoint': endpoint, 'initialize': True, 'journal_id': None,
                 'sources': {**common, 'workforce_generation': WGEN}, 'recovery': None, 'clock': None}
    orders = {'schema': 'dfmcp.semantic-work-order-development/1', 'directory': str(paths['orders']),
              'endpoint': endpoint, 'initialize': True, 'sources': {**common, 'work_order_generation': OGEN,
              'progress_generation': 70, 'operations_generation': 71}, 'recovery': None, 'clock': None}
    parent = {'schema': 'dfmcp.combined-production-development/1', 'directory': str(paths['parent']),
              'initialize': True, 'recovery': None}
    env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    env.update({'DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION': '1',
        'DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE': '1', 'DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR': '1',
        'DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS': '1', 'DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION': '1',
        'DFMCP_COMBINED_PRODUCTION_CONFIG': json.dumps(parent),
        'DFMCP_SEMANTIC_WORKFORCE_CONFIG': json.dumps(workforce),
        'DFMCP_SEMANTIC_WORK_ORDER_CONFIG': json.dumps(orders)})
    for name in ('DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN', 'DFMCP_SEMANTIC_WORKFORCE_TOKEN',
                 'DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN', 'DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN',
                 'DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN', 'DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN'):
        env[name] = TOKEN.decode()
    return env


class CombinedProduction(unittest.TestCase):
    def setUp(self):
        self.directory = LOGS / self._testMethodName
        self.directory.mkdir(mode=0o700)
        self.peer = Peer(self.directory)
        self.env = environment(self.directory, self.peer.endpoint)
        self.client = stdio.Client(self.directory, {}, env=self.env)

    def tearDown(self):
        try: self.client.close()
        finally: self.peer.close()

    def enable_clock(self):
        self.client.close()
        for name in ('DFMCP_COMBINED_PRODUCTION_CONFIG', 'DFMCP_SEMANTIC_WORK_ORDER_CONFIG'):
            config = json.loads(self.env[name])
            config['clock'] = {'generation': CLOCK_GENERATION}
            self.env[name] = json.dumps(config)
        self.env['DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK'] = '1'
        self.env['DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN'] = TOKEN.decode()
        self.client = stdio.Client(self.directory, {}, suffix='clock', env=self.env)

    def enable_inventory_clock(self):
        self.client.close()
        for name in ('DFMCP_COMBINED_PRODUCTION_CONFIG', 'DFMCP_SEMANTIC_WORK_ORDER_CONFIG'):
            config = json.loads(self.env[name])
            config['inventory_clock'] = {'generation': INVENTORY_CLOCK_GENERATION}
            self.env[name] = json.dumps(config)
        self.env['DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK'] = '1'
        self.env['DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN'] = TOKEN.decode()
        self.client = stdio.Client(self.directory, {}, suffix='inventory-clock', env=self.env)

    def inventory_plan(self, stable_samples=1, max_game_ticks=0):
        return self.production_plan(stable_samples, max_game_ticks, inventory=True)

    def order_plan(self, stable_samples=1):
        return self.production_plan(stable_samples, inventory=False)

    def production_plan(self, stable_samples=1, max_game_ticks=0, *, inventory):
        options = {'max_wall_millis': 60000}
        if max_game_ticks:
            options['max_game_ticks'] = max_game_ticks
        opened = self.client.open(**options)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': stable_samples}
        if inventory:
            request['objective'] = {'kind': 'observed_inventory_minimum', 'item_type': 'BED',
                                    'minimum_stack_units': 1}
        current = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(current['ok'], current)
        original = current['original_plan']
        self.assertEqual(len(original['steps']), 2)
        self.assertEqual(original['steps'][1]['depends_on'], [0])
        for index in (0, 1):
            prepared = self.client.execute_next(current)
            self.assertTrue(prepared['ok'], prepared)
            self.assertEqual(prepared['review']['step_id'], index)
            self.assertEqual(prepared['review']['idempotency_key'], original['steps'][index]['key'])
            current = self.client.execute_next(prepared)
            self.assertTrue(current['ok'], current)
        self.assertTrue(current['frontier']['all_applied'])
        self.assertFalse(current['original_goal_proven'])
        self.assertIsNone(current['current_parent_goal'])
        selected = 'selected_inventory_goal' if inventory else 'selected_order_goal'
        self.assertFalse(current[selected]['proven_at_anchor'])
        self.assertIsNone(current['selected_order_goal' if inventory else 'selected_inventory_goal'])
        return sid, original, current

    def goal_sample(self, sid, tick, *, inventory=True):
        self.peer.goal_tick(tick)
        result = self.client.tool('wait', session_id=sid)
        self.assertTrue(result['ok'], result)
        self.assertFalse(result['current_state_proven'])
        self.assertFalse(result['clock_advanced'])
        self.assertFalse(result['automatic_redispatch'])
        self.assertFalse(result['runtime_admitted'])
        parent = result['current_parent_goal']
        selected = 'selected_inventory_goal' if inventory else 'selected_order_goal'
        scope = 'complete_original_labor_and_inventory_goal' if inventory else 'complete_original_labor_and_order_goal'
        self.assertEqual(parent['scope'], scope)
        self.assertEqual(parent['anchor'], result['anchor'])
        self.assertEqual(parent['selected_anchor'], result[selected]['anchor'])
        for field in ('fortress_id', 'epoch', 'sequence', 'tick'):
            self.assertEqual(parent['anchor'][field], parent['selected_anchor'][field])
        self.assertNotEqual(parent['anchor']['state_hash'], parent['selected_anchor']['state_hash'])
        self.assertNotEqual(parent['source_digest'], result[selected]['source_digest'])
        self.assertEqual(len(parent['workforce_source_digest']), 64)
        self.assertEqual(parent['original_goal_proven'], result['original_goal_proven'])
        self.assertFalse(parent['historical_labor_receipt_is_current_evidence'])
        self.assertFalse(parent['restored_from_history'])
        self.assertEqual(parent['production_attribution'], 'unknown')
        labor = self.peer.workforce_labor is not False
        self.assertEqual(parent['labor'], [{'unit_id': '43', 'labor': 'MINE',
            'requested_enabled': True, 'observed_enabled': labor,
            'current_truth': 'true' if labor else 'false'}])
        self.assert_single_dispatch()
        return result

    def test_default_order_goal_tracks_outstanding_completion_stability_and_current_labor(self):
        sid, original, created = self.order_plan(stable_samples=2)
        goal = created['selected_order_goal']
        self.assertEqual(goal['kind'], 'retained_order_completion')
        self.assertEqual(goal['step_id'], 1)
        self.assertEqual(goal['required_order_state'], {'amount_total': 1, 'amount_remaining': 0,
                                                       'validated': True, 'active': False})
        self.assertFalse(goal['disappearance_proves_completion'])
        self.assertEqual(goal['deadline_tick'], 300)
        outstanding = self.goal_sample(sid, 125, inventory=False)
        self.assertEqual(outstanding['selected_order_goal']['current_truth'], 'false')
        self.assertEqual(outstanding['current_parent_goal']['current_truth'], 'false')
        self.assertFalse(outstanding['original_goal_proven'])
        self.assertEqual(outstanding['agent_turn']['recommendations'][0]['tool'], 'fortress_wait')
        self.peer.order_remaining = 0
        self.peer.order_status = 1  # validated, inactive
        for tick in (130, 130):
            pending = self.goal_sample(sid, tick, inventory=False)
            self.assertEqual(pending['selected_order_goal']['current_truth'], 'true')
            self.assertEqual(pending['current_parent_goal']['current_truth'], 'true')
            self.assertFalse(pending['original_goal_proven'])
        achieved = self.goal_sample(sid, 135, inventory=False)
        self.assertTrue(achieved['selected_order_goal']['proven_at_anchor'])
        self.assertTrue(achieved['original_goal_proven'])
        self.assertTrue(achieved['agent_turn']['briefing']['selected_order_goal_proven_at_anchor'])
        self.assertFalse(achieved['agent_turn']['briefing']['selected_inventory_goal_proven_at_anchor'])
        self.assertEqual(achieved['agent_turn']['recommendations'], [])
        self.assertEqual(achieved['selected_order_goal']['order_entity_id'], goal['order_entity_id'])
        self.assert_single_original_effects(achieved, original)
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 5
        reversed_labor = self.goal_sample(sid, 140, inventory=False)
        self.assertTrue(reversed_labor['selected_order_goal']['proven_at_anchor'])
        self.assertEqual(reversed_labor['current_parent_goal']['current_truth'], 'false')
        self.assertFalse(reversed_labor['original_goal_proven'])
        self.assert_single_original_effects(reversed_labor, original)

    def test_default_order_retirement_and_preexisting_stock_cannot_prove_completion(self):
        sid, original, _ = self.order_plan()
        self.peer.order_retired = True
        self.peer.inventory_stack = 999
        current = self.goal_sample(sid, 125, inventory=False)
        selected = current['selected_order_goal']
        self.assertEqual(selected['current_truth'], 'unknown')
        self.assertEqual(current['current_parent_goal']['current_truth'], 'unknown')
        self.assertFalse(selected['receipt_linked'])
        self.assertFalse(selected['proven_at_anchor'])
        self.assertIsNone(selected['first_satisfied_anchor'])
        self.assertFalse(current['original_goal_proven'])
        self.assertEqual(selected['parent_terminal_evidence_scope'], 'selected_order_projection_only')
        self.assertNotIn('preexisting_items_included', selected)
        self.assert_single_original_effects(current, original)

    def test_default_order_history_reopens_without_restoring_current_completion(self):
        sid, original, _ = self.order_plan()
        self.peer.order_remaining = 0
        self.peer.order_status = 1
        achieved = self.goal_sample(sid, 125, inventory=False)
        self.assertTrue(achieved['original_goal_proven'])
        first = achieved['selected_order_goal']['first_satisfied_anchor']
        self.assertIsNotNone(first)
        closed = self.client.tool('cancel', session_id=sid, scope='session')
        self.assertTrue(closed['ok'], closed)
        self.assertFalse(closed['retained_reference']['selected_order_goal']['proven_at_anchor'])
        self.client.close()
        before = custody_files(self.directory)
        calls = list(self.peer.calls)
        env = reopen_environment(self.env, achieved['recovery'], achieved['native_custody']['workforce_journal_id'])
        del env['DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR']
        del env['DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION']
        self.client = stdio.Client(self.directory, {}, suffix='order-goal-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertIsNone(offline['selected_order_goal']['current_truth'])
        self.assertFalse(offline['selected_order_goal']['proven_at_anchor'])
        self.assertEqual(offline['selected_order_goal']['first_satisfied_anchor'], first)
        self.assertEqual(offline['selected_order_goal']['required_order_state'],
                         achieved['selected_order_goal']['required_order_state'])
        self.assertEqual(before, custody_files(self.directory))
        self.assertEqual(calls, self.peer.calls)
        self.assertEqual(offline['original_plan'], original)
        self.client.close()
        # A finite order cannot regain remaining work after completion; that
        # would correctly trip the retained source/configuration fence. Its
        # legitimate retirement must instead withdraw current completion while
        # preserving the historical qualified achievement.
        self.peer.order_retired = True
        env = reopen_environment(self.env, offline['recovery'], achieved['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='order-goal-reopened', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000)
        self.assertTrue(reopened['ok'], reopened)
        self.assertFalse(reopened['original_goal_proven'])
        current = self.goal_sample(reopened['agent_turn']['session_id'], 130, inventory=False)
        self.assertEqual(current['selected_order_goal']['current_truth'], 'unknown')
        self.assertEqual(current['current_parent_goal']['current_truth'], 'unknown')
        self.assertFalse(current['selected_order_goal']['receipt_linked'])
        self.assertFalse(current['selected_order_goal']['proven_at_anchor'])
        self.assertFalse(current['original_goal_proven'])
        self.assertEqual(current['selected_order_goal']['first_satisfied_anchor'], first)
        self.assert_single_original_effects(current, original)

    def assert_single_dispatch(self):
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)
        self.assertEqual(sum(op == 'CommitOrder' for _, op in self.peer.calls), 1)
        self.assertFalse(any('Run' in op for _, op in self.peer.calls))

    def assert_single_original_effects(self, result, original):
        self.assertEqual(result['original_plan'], original)
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)
        self.assertEqual(sum(op == 'CommitOrder' for _, op in self.peer.calls), 1)
        self.assertEqual([entry['idempotency_key'] for entry in result['frontier']['entries']],
                         [step['key'] for step in original['steps']])

    def prepare_clock(self, sid, view, ticks=5):
        request = {'kind': 'run', 'original_paired_seal': view['clock']['original_paired_seal'],
                   'game_ticks': ticks, 'wall_millis': 1000}
        return self.client.tool('plan', session_id=sid, request=json.dumps(request))

    def prepare_inventory_clock(self, sid, view, ticks=5):
        request = {'kind': 'inventory_run',
                   'original_paired_seal': view['inventory_clock']['original_paired_seal'],
                   'game_ticks': ticks, 'wall_millis': 1000}
        return self.client.tool('plan', session_id=sid, request=json.dumps(request))

    def inventory_clock_sample(self, sid):
        current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(current['ok'], current)
        self.assertEqual(current['current_parent_goal']['current_truth'], 'true')
        self.assertEqual(current['selected_inventory_goal']['current_truth'], 'true')
        self.assertFalse(current['original_goal_proven'])
        self.assertEqual(current['selected_inventory_goal']['deadline_tick'], 300)
        return current

    def test_retired_order_inventory_run_finishes_the_original_parent_stability(self):
        self.enable_inventory_clock()
        sid, original, created = self.inventory_plan(stable_samples=2, max_game_ticks=5)
        self.assertTrue(created['inventory_clock_control_enabled'])
        self.peer.order_retired = True
        refused = self.prepare_inventory_clock(sid, created)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.peer.inventory_clock_plans, {})
        current = self.inventory_clock_sample(sid)
        self.assertFalse(current['selected_inventory_goal']['receipt_linked'])
        recommendation = current['agent_turn']['recommendations'][0]
        self.assertEqual(recommendation['tool'], 'fortress_plan')
        self.assertEqual(json.loads(recommendation['arguments']['request'])['kind'], 'inventory_run')
        # The original tick is the obligation baseline. The two qualified
        # advancing samples, at125 and130, require two distinct reviewed slices.
        for expected_tick in (TICK + 5, TICK + 10):
            before_tick = self.peer.order_tick
            prepared = self.prepare_inventory_clock(sid, current)
            self.assertTrue(prepared['ok'], prepared)
            self.assertTrue(prepared['inventory_clock']['review_available'])
            self.assertEqual(self.peer.order_tick, before_tick)
            run = prepared['inventory_clock']['active_run']
            self.assertEqual(run['purpose'], 'advance_original_inventory_stability')
            self.assertFalse(prepared['inventory_clock']['original_deadline_renewed'])
            recommendation = prepared['agent_turn']['recommendations'][0]
            self.assertEqual(recommendation['arguments']['scope'], 'inventory_run')
            self.assertEqual(recommendation['arguments']['paired_seal'], run['review_seal'])
            committed = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                         paired_seal=run['review_seal'], confirmed=True)
            self.assertTrue(committed['ok'], committed)
            self.assertEqual(self.peer.order_tick, expected_tick)
            self.assertFalse(committed['original_goal_proven'])
            self.assertIsNone(committed['current_parent_goal'])
            self.assertTrue(committed['inventory_clock_invalidates_goal'])
            self.assertFalse(committed['selected_inventory_goal']['proven_at_anchor'])
            current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
            self.assertTrue(current['ok'], current)
            self.assertEqual(current['original_goal_proven'], expected_tick == TICK + 10)
        achieved = current
        self.assertTrue(achieved['original_goal_proven'])
        self.assertFalse(achieved['current_state_proven'])
        self.assertFalse(achieved['inventory_clock_invalidates_goal'])
        self.assertFalse(achieved['selected_inventory_goal']['receipt_linked'])
        self.assertEqual(achieved['selected_inventory_goal']['deadline_tick'], 300)
        self.assertEqual(achieved['current_parent_goal']['production_attribution'], 'unknown')
        self.assertTrue(achieved['agent_turn']['briefing']['original_goal_proven'])
        self.assert_single_original_effects(achieved, original)
        self.assertEqual(len(self.peer.inventory_clock_commits), 2)
        self.assertEqual(len(set(self.peer.inventory_clock_commits)), 2)
        self.assertEqual(self.peer.clock_commits, [])
        refused = self.prepare_inventory_clock(sid, achieved)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(len(self.peer.inventory_clock_commits), 2)

    def assert_inventory_clock_late_change_refuses_unpause(self, change):
        self.enable_inventory_clock()
        sid, original, _ = self.inventory_plan(stable_samples=3, max_game_ticks=5)
        self.peer.order_retired = True
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_inventory_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        seal = prepared['inventory_clock']['active_run']['review_seal']
        # The native journal's commit preflight has already followed a successful
        # foreground parent sample. Its next edge persists DispatchStarted and
        # invokes the fresh full-parent guard before native CommitRun.
        self.peer.after_inventory_clock_observe = change
        refused = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                   paired_seal=seal, confirmed=True)
        self.assertFalse(refused['ok'], refused)
        self.assertIsNone(self.peer.after_inventory_clock_observe)
        retained = refused['retained_reference']
        self.assertIsNone(retained['current_parent_goal'])
        self.assertFalse(retained['selected_inventory_goal']['proven_at_anchor'])
        self.assertTrue(retained['inventory_clock']['unresolved'])
        self.assertEqual(retained['inventory_clock']['active_run']['state'], 'dispatch_started')
        self.assertEqual(self.peer.inventory_clock_commits, [])
        self.assertEqual(len(self.peer.inventory_clock_plans), 1)
        self.assertEqual(self.peer.order_tick, TICK)
        retried = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                   paired_seal=seal, confirmed=True)
        self.assertFalse(retried['ok'], retried)
        self.assertEqual(self.peer.inventory_clock_commits, [])
        inspected = self.client.tool('query', session_id=sid)
        self.assertTrue(inspected['ok'], inspected)
        self.assert_single_original_effects(inspected, original)

    def test_inventory_clock_rechecks_stock_after_the_dispatch_marker(self):
        self.assert_inventory_clock_late_change_refuses_unpause(
            lambda: setattr(self.peer, 'inventory_stack', 0))

    def test_inventory_clock_rechecks_current_labor_after_the_dispatch_marker(self):
        def reverse_labor():
            self.peer.workforce_labor = False
            self.peer.workforce_sequence = 5
        self.assert_inventory_clock_late_change_refuses_unpause(reverse_labor)

    def test_lost_inventory_clock_reply_reconciles_original_key_without_unpause_replay(self):
        self.enable_inventory_clock()
        sid, original, created = self.inventory_plan(stable_samples=2, max_game_ticks=5)
        self.peer.order_retired = True
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_inventory_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        seal = prepared['inventory_clock']['active_run']['review_seal']
        self.peer.hold_inventory_clock = True
        self.peer.drop_inventory_clock_reply = True
        lost = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                paired_seal=seal, confirmed=True)
        self.assertFalse(lost['ok'], lost)
        self.assertIsNone(lost['retained_reference']['current_parent_goal'])
        self.assertEqual(len(self.peer.inventory_clock_commits), 1)
        original_key = self.peer.inventory_clock_commits[0]
        self.client.close(crash=True)
        env = reopen_environment(self.env, lost['retained_reference']['recovery'],
                                 created['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='inventory-reconcile', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000, max_game_ticks=5)
        self.assertTrue(reopened['ok'], reopened)
        self.assertFalse(reopened['inventory_clock']['review_available'])
        self.assertFalse(reopened['inventory_clock_control_enabled'])
        self.assertIsNone(reopened['current_parent_goal'])
        sid = reopened['agent_turn']['session_id']
        reads = sum(plugin in ('dfmcp_work_order_progress_v1_12', 'dfmcp_operations_v1_3')
                    and op == 'ReadObservation' for plugin, op in self.peer.calls)
        pending = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(pending['ok'], pending)
        self.assertTrue(pending['inventory_clock']['unresolved'])
        self.assertEqual(reads, sum(plugin in ('dfmcp_work_order_progress_v1_12', 'dfmcp_operations_v1_3')
                                   and op == 'ReadObservation' for plugin, op in self.peer.calls))
        self.peer.finish_inventory_clock(original_key)
        settled = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(settled['ok'], settled)
        self.assertFalse(settled['inventory_clock']['unresolved'])
        self.assertEqual(settled['current_parent_goal']['current_truth'], 'true')
        self.assertFalse(settled['original_goal_proven'])
        self.assert_single_original_effects(settled, original)
        denied = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                  paired_seal=seal, confirmed=True)
        self.assertFalse(denied['ok'], denied)
        self.assertEqual(self.peer.inventory_clock_commits, [original_key])
        self.assertEqual(len(self.peer.inventory_clock_plans), 1)

    def test_prepared_and_running_order_clock_block_inventory_run(self):
        self.enable_clock()
        self.enable_inventory_clock()
        sid, _, _ = self.inventory_plan(stable_samples=3, max_game_ticks=10)
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        refused = self.prepare_inventory_clock(sid, prepared)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.peer.inventory_clock_plans, {})
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        self.peer.hold_clock = True
        running = self.client.tool('commit', session_id=sid, scope='run',
                                   paired_seal=prepared['clock']['active_run']['review_seal'], confirmed=True)
        self.assertTrue(running['ok'], running)
        self.assertTrue(running['clock']['unresolved'])
        refused = self.prepare_inventory_clock(sid, running)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.peer.inventory_clock_plans, {})
        self.assertEqual(self.peer.inventory_clock_commits, [])
        cancelled = self.client.tool('cancel', session_id=sid, scope='run')
        self.assertTrue(cancelled['ok'], cancelled)
        self.assertFalse(cancelled['clock']['pending'])
        self.assertEqual(len(self.peer.clock_commits), 1)

    def test_prepared_and_running_inventory_clock_block_order_run(self):
        self.enable_clock()
        self.enable_inventory_clock()
        sid, _, _ = self.inventory_plan(stable_samples=3, max_game_ticks=10)
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_inventory_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        refused = self.prepare_clock(sid, prepared)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.peer.clock_plans, {})
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_inventory_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        self.peer.hold_inventory_clock = True
        running = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                   paired_seal=prepared['inventory_clock']['active_run']['review_seal'],
                                   confirmed=True)
        self.assertTrue(running['ok'], running)
        self.assertTrue(running['inventory_clock']['unresolved'])
        refused = self.prepare_clock(sid, running)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.peer.clock_plans, {})
        self.assertEqual(self.peer.clock_commits, [])
        cancelled = self.client.tool('cancel', session_id=sid, scope='inventory_run')
        self.assertTrue(cancelled['ok'], cancelled)
        self.assertFalse(cancelled['inventory_clock']['pending'])
        self.assertEqual(len(self.peer.inventory_clock_commits), 1)

    def test_retained_inventory_clock_reopens_offline_without_review_or_native_contact(self):
        self.enable_inventory_clock()
        sid, original, created = self.inventory_plan(stable_samples=2, max_game_ticks=5)
        self.peer.order_retired = True
        current = self.inventory_clock_sample(sid)
        prepared = self.prepare_inventory_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        seal = prepared['inventory_clock']['active_run']['review_seal']
        self.client.close(crash=True)
        env = reopen_environment(self.env, prepared['recovery'],
                                 created['native_custody']['workforce_journal_id'])
        for gate in ('DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK',
                     'DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR', 'DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION'):
            del env[gate]
        before, calls = custody_files(self.directory), list(self.peer.calls)
        self.client = stdio.Client(self.directory, {}, suffix='inventory-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertFalse(offline['inventory_clock']['review_available'])
        self.assertFalse(offline['inventory_clock_control_enabled'])
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertEqual(offline['original_plan'], original)
        self.assertEqual(offline['inventory_clock']['run_count'], 1)
        denied = self.client.tool('commit', session_id=offline['agent_turn']['session_id'],
                                  scope='inventory_run', paired_seal=seal, confirmed=True)
        self.assertFalse(denied['ok'], denied)
        self.assertEqual(before, custody_files(self.directory))
        self.assertEqual(calls, self.peer.calls)
        self.assertEqual(self.peer.inventory_clock_commits, [])
        self.client.close()
        path = self.directory / 'orders' / 'inventory-clock.journal'
        self.assertTrue(path.exists())
        path.rename(path.with_name('inventory-clock.journal.retained-test-copy'))
        self.client = stdio.Client(self.directory, {}, suffix='inventory-missing', env=env)
        rejected = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertFalse(rejected['ok'], rejected)
        self.assertEqual(calls, self.peer.calls)

    def test_inventory_clock_safety_cancel_survives_original_deadline_and_lost_stock(self):
        self.enable_inventory_clock()
        sid, original, _ = self.inventory_plan(stable_samples=2, max_game_ticks=5)
        self.peer.order_retired = True
        prepared = self.prepare_inventory_clock(sid, self.inventory_clock_sample(sid))
        self.assertTrue(prepared['ok'], prepared)
        self.peer.hold_inventory_clock = True
        running = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                   paired_seal=prepared['inventory_clock']['active_run']['review_seal'],
                                   confirmed=True)
        self.assertTrue(running['ok'], running)
        self.assertTrue(running['inventory_clock']['unresolved'])
        self.peer.goal_tick(301)
        self.peer.inventory_stack = 0
        reads = [call for call in self.peer.calls if call[0] in
                 ('dfmcp_operations_v1_3', 'dfmcp_work_order_progress_v1_12')]
        cancelled = self.client.tool('cancel', session_id=sid, scope='inventory_run')
        self.assertTrue(cancelled['ok'], cancelled)
        self.assertFalse(cancelled['inventory_clock']['pending'])
        self.assertFalse(cancelled['inventory_clock']['unresolved'])
        self.assertFalse(cancelled['original_goal_proven'])
        self.assertIsNone(cancelled['current_parent_goal'])
        self.assertEqual(cancelled['inventory_clock']['runs'][0]['native']['reason'], 'cancelled')
        self.assertEqual(cancelled['inventory_clock']['runs'][0]['native']['observed_tick'], 301)
        self.assertEqual(reads, [call for call in self.peer.calls if call[0] in
            ('dfmcp_operations_v1_3', 'dfmcp_work_order_progress_v1_12')])
        current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(current['ok'], current)
        self.assertFalse(current['original_goal_proven'])
        self.assertEqual(current['selected_inventory_goal']['deadline_tick'], 300)
        self.assertTrue(current['selected_inventory_goal']['terminal_failure'])
        self.assert_single_original_effects(current, original)
        self.assertEqual(len(self.peer.inventory_clock_commits), 1)
        refused = self.prepare_inventory_clock(sid, current)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(len(self.peer.inventory_clock_plans), 1)

    def test_rejected_inventory_commit_capture_retains_consumed_tick_floor(self):
        self.enable_inventory_clock()
        sid, original, created = self.inventory_plan(stable_samples=2, max_game_ticks=5)
        self.peer.order_retired = True
        prepared = self.prepare_inventory_clock(sid, self.inventory_clock_sample(sid))
        self.assertTrue(prepared['ok'], prepared)
        # Only the native clock preflight is consulted before this mismatch
        # refuses dispatch. Its complete tick125 observation must still survive.
        self.peer.order_tick = 125
        refused = self.client.tool('commit', session_id=sid, scope='inventory_run',
                                   paired_seal=prepared['inventory_clock']['active_run']['review_seal'],
                                   confirmed=True)
        self.assertFalse(refused['ok'], refused)
        self.assertEqual(self.peer.inventory_clock_commits, [])
        consumed_floor = refused['retained_reference']['observation_tick_floor']
        self.peer.order_tick = TICK
        rolled_back = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertGreaterEqual(consumed_floor, 125,
                                'a rejected complete native capture must raise the original owner floor')
        self.assertFalse(rolled_back['ok'], rolled_back)
        self.assertIsNone(rolled_back['retained_reference']['current_parent_goal'])
        self.assertGreaterEqual(rolled_back['retained_reference']['observation_tick_floor'], 125)
        inspected = self.client.tool('query', session_id=sid)
        self.assertTrue(inspected['ok'], inspected)
        self.assert_single_original_effects(inspected, original)
        self.assertEqual(self.peer.inventory_clock_commits, [])
        self.assertGreaterEqual(inspected['recovery']['frontier']['authority_tick_floor'], 125)
        self.client.close(crash=True)
        env = reopen_environment(self.env, inspected['recovery'],
                                 created['native_custody']['workforce_journal_id'])
        before, calls = custody_files(self.directory), list(self.peer.calls)
        self.client = stdio.Client(self.directory, {}, suffix='floor-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertGreaterEqual(offline['observation_tick_floor'], 125)
        self.assertEqual(offline['inventory_clock']['high_tick'], TICK)
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertEqual(before, custody_files(self.directory))
        self.assertEqual(calls, self.peer.calls)
        self.client.close()
        env = reopen_environment(self.env, offline['recovery'],
                                 created['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='floor-reconcile', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000)
        self.assertTrue(reopened['ok'], reopened)
        self.assertFalse(reopened['inventory_clock']['review_available'])
        self.assertGreaterEqual(reopened['observation_tick_floor'], 125)
        rolled_back = self.client.tool('wait', session_id=reopened['agent_turn']['session_id'],
                                       max_wall_millis=60000)
        self.assertFalse(rolled_back['ok'], rolled_back)
        self.assertIsNone(rolled_back['retained_reference']['current_parent_goal'])
        self.assertFalse(rolled_back['retained_reference']['selected_inventory_goal']['proven_at_anchor'])
        self.assertGreaterEqual(rolled_back['retained_reference']['observation_tick_floor'], 125)
        self.assertEqual(self.peer.inventory_clock_commits, [])

    def test_reviewed_clock_advances_original_inventory_to_complete_parent_proof(self):
        self.enable_clock()
        self.peer.inventory_stack = 0
        sid, original, current = self.inventory_plan(stable_samples=2, max_game_ticks=10)
        self.assertTrue(current['clock_control_enabled'])
        for expected_tick in (125, 130):
            before_tick = self.peer.order_tick
            prepared = self.prepare_clock(sid, current)
            self.assertTrue(prepared['ok'], prepared)
            self.assertTrue(prepared['clock']['review_available'])
            self.assertEqual(self.peer.order_tick, before_tick)
            run = prepared['clock']['active_run']
            next_action = prepared['agent_turn']['recommendations'][0]
            self.assertEqual(next_action['tool'], 'fortress_commit')
            self.assertEqual(next_action['arguments']['scope'], 'run')
            self.assertEqual(next_action['arguments']['paired_seal'], run['review_seal'])
            committed = self.client.tool('commit', session_id=sid, scope='run',
                                         paired_seal=run['review_seal'], confirmed=True)
            self.assertTrue(committed['ok'], committed)
            self.assertFalse(committed['original_goal_proven'])
            self.assertIsNone(committed['current_parent_goal'])
            self.assertFalse(committed['selected_inventory_goal']['proven_at_anchor'])
            self.assertEqual(self.peer.order_tick, expected_tick)
            current = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
            self.assertTrue(current['ok'], current)
            self.assertEqual(current['selected_inventory_goal']['deadline_tick'], 300)
            self.assertEqual(current['original_goal_proven'], expected_tick == 130)
            if expected_tick == 125:
                next_action = current['agent_turn']['recommendations'][0]
                self.assertEqual(next_action['tool'], 'fortress_plan')
                request = json.loads(next_action['arguments']['request'])
                self.assertEqual(request['kind'], 'run')
                self.assertEqual(request['game_ticks'], 5)
        self.assertTrue(current['agent_turn']['briefing']['original_goal_proven'])
        self.assertEqual(len(self.peer.clock_commits), 2)
        self.assertEqual(len(set(self.peer.clock_commits)), 2)
        self.assert_single_original_effects(current, original)
        committed_count = len(self.peer.clock_commits)
        rejected = self.prepare_clock(sid, current)
        self.assertFalse(rejected['ok'], rejected)
        self.assertEqual(len(self.peer.clock_commits), committed_count)

    def test_lost_clock_reply_reopens_and_reconciles_without_repeating_unpause(self):
        self.enable_clock()
        self.peer.inventory_stack = 0
        sid, original, current = self.inventory_plan(stable_samples=2, max_game_ticks=10)
        prepared = self.prepare_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        seal = prepared['clock']['active_run']['review_seal']
        self.peer.hold_clock = True
        self.peer.drop_clock_reply = True
        lost = self.client.tool('commit', session_id=sid, scope='run', paired_seal=seal, confirmed=True)
        self.assertFalse(lost['ok'], lost)
        self.assertIsNone(lost['retained_reference']['current_parent_goal'])
        self.assertEqual(len(self.peer.clock_commits), 1)
        self.client.close(crash=True)
        env = reopen_environment(self.env, lost['retained_reference']['recovery'],
                                 current['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='clock-reconcile', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000, max_game_ticks=10)
        self.assertTrue(reopened['ok'], reopened)
        self.assertFalse(reopened['clock']['review_available'])
        sid = reopened['agent_turn']['session_id']
        reads = sum(op == 'ReadObservation' for plugin, op in self.peer.calls
                    if plugin == 'dfmcp_work_order_progress_v1_12')
        pending = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(pending['ok'], pending)
        self.assertTrue(pending['clock']['unresolved'])
        self.assertEqual(reads, sum(op == 'ReadObservation' for plugin, op in self.peer.calls
                                  if plugin == 'dfmcp_work_order_progress_v1_12'))
        self.peer.finish_clock(self.peer.clock_commits[0])
        settled = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertTrue(settled['ok'], settled)
        self.assertFalse(settled['clock']['unresolved'])
        self.assertEqual(settled['current_parent_goal']['current_truth'], 'true')
        self.assertFalse(settled['original_goal_proven'])
        self.assert_single_original_effects(settled, original)
        denied = self.client.tool('commit', session_id=sid, scope='run', paired_seal=seal, confirmed=True)
        self.assertFalse(denied['ok'], denied)
        self.assertEqual(len(self.peer.clock_commits), 1)

    def test_reopened_prepared_clock_has_no_review_and_missing_journal_refuses(self):
        self.enable_clock()
        self.peer.inventory_stack = 0
        sid, _, current = self.inventory_plan(stable_samples=2, max_game_ticks=10)
        prepared = self.prepare_clock(sid, current)
        self.assertTrue(prepared['ok'], prepared)
        old_seal = prepared['clock']['active_run']['review_seal']
        self.client.close(crash=True)
        env = reopen_environment(self.env, prepared['recovery'], current['native_custody']['workforce_journal_id'])
        before, calls = custody_files(self.directory), list(self.peer.calls)
        self.client = stdio.Client(self.directory, {}, suffix='clock-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertFalse(offline['clock']['review_available'])
        self.assertFalse(offline['original_goal_proven'])
        self.assertEqual(before, custody_files(self.directory))
        self.assertEqual(calls, self.peer.calls)
        denied = self.client.tool('commit', session_id=offline['agent_turn']['session_id'],
                                  scope='run', paired_seal=old_seal, confirmed=True)
        self.assertFalse(denied['ok'], denied)
        self.assertEqual(self.peer.clock_commits, [])
        self.client.close()
        path = self.directory / 'orders' / 'clock.journal'
        self.assertTrue(path.exists())
        path.rename(path.with_name('clock.journal.retained-test-copy'))
        self.client = stdio.Client(self.directory, {}, suffix='clock-missing', env=env)
        rejected = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertFalse(rejected['ok'], rejected)
        self.assertEqual(calls, self.peer.calls)

    def test_current_parent_requires_labor_and_original_stability_after_order_retirement(self):
        sid, original, _ = self.inventory_plan(stable_samples=2)
        first = self.goal_sample(sid, 125)
        self.assertEqual(first['current_parent_goal']['current_truth'], 'true')
        self.assertFalse(first['current_parent_goal']['original_goal_proven'])
        self.assertFalse(first['original_goal_proven'])
        duplicate = self.goal_sample(sid, 125)
        self.assertFalse(duplicate['original_goal_proven'])
        self.peer.order_retired = True
        achieved = self.goal_sample(sid, 130)
        self.assertTrue(achieved['original_goal_proven'])
        self.assertFalse(achieved['current_state_proven'])
        self.assertTrue(achieved['current_parent_goal']['original_goal_proven'])
        selected = achieved['selected_inventory_goal']
        self.assertTrue(selected['proven_at_anchor'])
        self.assertFalse(selected['original_goal_proven'])
        self.assertFalse(selected['receipt_linked'])
        self.assertIn('Unknown', selected['physical_work'])
        self.assertEqual(achieved['current_parent_goal']['selected_anchor'], selected['anchor'])
        self.assertEqual(achieved['current_parent_goal']['anchor'], achieved['anchor'])
        self.assertNotEqual(selected['anchor']['state_hash'], achieved['anchor']['state_hash'])
        self.assertTrue(achieved['agent_turn']['briefing']['original_goal_proven'])
        self.assertNotIn('fortress_wait', [r['tool'] for r in achieved['agent_turn']['recommendations']])
        calls = list(self.peer.calls)
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertEqual(queried['current_parent_goal'], achieved['current_parent_goal'])
        self.assertEqual(self.peer.calls, calls)
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 5
        reversed_goal = self.goal_sample(sid, 135)
        self.assertEqual(reversed_goal['current_parent_goal']['current_truth'], 'false')
        self.assertTrue(reversed_goal['selected_inventory_goal']['proven_at_anchor'])
        self.assertFalse(reversed_goal['original_goal_proven'])
        self.assertFalse(reversed_goal['agent_turn']['briefing']['original_goal_proven'])
        self.assert_single_original_effects(reversed_goal, original)

    def test_parent_bracket_failure_withdraws_proof_and_breaks_unfinished_stability(self):
        sid, original, _ = self.inventory_plan(stable_samples=3)
        first = self.goal_sample(sid, 125)
        self.assertFalse(first['original_goal_proven'])
        def reverse_during_inventory():
            self.peer.workforce_labor = False
            self.peer.workforce_sequence = 5
        self.peer.after_operations_read = reverse_during_inventory
        self.peer.goal_tick(130)
        rejected = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertFalse(rejected['ok'], rejected)
        self.assertFalse(rejected['original_goal_proven'])
        self.assertIsNone(rejected['retained_reference']['current_parent_goal'])
        self.assertFalse(rejected['retained_reference']['selected_inventory_goal']['proven_at_anchor'])
        inspected = self.client.tool('query', session_id=sid)
        self.assertTrue(inspected['ok'], inspected)
        self.assertIsNone(inspected['current_parent_goal'])
        self.assertFalse(inspected['selected_inventory_goal']['proven_at_anchor'])
        self.peer.workforce_labor = True
        self.peer.workforce_sequence = 6
        for tick in (135, 140):
            pending = self.goal_sample(sid, tick)
            self.assertEqual(pending['current_parent_goal']['current_truth'], 'true')
            self.assertFalse(pending['original_goal_proven'])
        achieved = self.goal_sample(sid, 145)
        self.assertTrue(achieved['original_goal_proven'])
        self.assert_single_original_effects(achieved, original)

    def test_source_generation_drift_withdraws_current_parent_proof(self):
        sid, original, _ = self.inventory_plan(stable_samples=2)
        self.goal_sample(sid, 125)
        achieved = self.goal_sample(sid, 130)
        self.assertTrue(achieved['original_goal_proven'])
        first = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.assertIsNotNone(first)
        self.peer.workforce_generation += 1
        self.peer.goal_tick(135)
        rejected = self.client.tool('wait', session_id=sid, max_wall_millis=60000)
        self.assertFalse(rejected['ok'], rejected)
        self.assertFalse(rejected['original_goal_proven'])
        self.assertIsNone(rejected['retained_reference']['current_parent_goal'])
        self.assertFalse(rejected['retained_reference']['selected_inventory_goal']['proven_at_anchor'])
        calls = list(self.peer.calls)
        inspected = self.client.tool('query', session_id=sid)
        self.assertTrue(inspected['ok'], inspected)
        self.assertFalse(inspected['original_goal_proven'])
        self.assertIsNone(inspected['current_parent_goal'])
        selected = inspected['selected_inventory_goal']
        self.assertFalse(selected['proven_at_anchor'])
        for field in ('current_truth', 'anchor', 'source_digest', 'parent_terminal_truth'):
            self.assertIsNone(selected[field])
        self.assertEqual(selected['first_satisfied_anchor'], first)
        self.assertEqual(calls, self.peer.calls)
        self.assert_single_original_effects(inspected, original)

    def test_reopen_keeps_inventory_achievement_without_restoring_parent_proof(self):
        sid, original, _ = self.inventory_plan(stable_samples=2)
        self.goal_sample(sid, 125)
        achieved = self.goal_sample(sid, 130)
        self.assertTrue(achieved['original_goal_proven'])
        first = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.assertIsNotNone(first)
        closed = self.client.tool('cancel', session_id=sid, scope='session')
        self.assertTrue(closed['ok'], closed)
        self.assertIsNone(closed['retained_reference']['current_parent_goal'])
        self.client.close()
        before = custody_files(self.directory)
        calls = list(self.peer.calls)
        env = reopen_environment(self.env, achieved['recovery'], achieved['native_custody']['workforce_journal_id'])
        # Historical native receipts cannot recreate operator write grants.
        del env['DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR']
        del env['DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION']
        self.client = stdio.Client(self.directory, {}, suffix='goal-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertFalse(offline['labor_control_enabled'])
        self.assertFalse(offline['production_control_enabled'])
        self.assertEqual(offline['original_plan'], original)
        self.assertIsNone(offline['selected_inventory_goal']['current_truth'])
        self.assertEqual(offline['selected_inventory_goal']['first_satisfied_anchor'], first)
        self.assertFalse(offline['selected_inventory_goal']['proven_at_anchor'])
        denied = self.client.tool('commit', session_id=offline['agent_turn']['session_id'],
                                  paired_seal='a' * 64, confirmed=True)
        self.assertFalse(denied['ok'], denied)
        self.assertEqual(before, custody_files(self.directory))
        self.assertEqual(calls, self.peer.calls)
        self.client.close()
        self.peer.workforce_labor = False
        self.peer.workforce_sequence = 5
        env = reopen_environment(self.env, offline['recovery'], achieved['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='goal-reopened', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000)
        self.assertTrue(reopened['ok'], reopened)
        self.assertFalse(reopened['original_goal_proven'])
        current = self.goal_sample(reopened['agent_turn']['session_id'], 135)
        self.assertEqual(current['current_parent_goal']['current_truth'], 'false')
        self.assertTrue(current['selected_inventory_goal']['proven_at_anchor'])
        self.assertFalse(current['original_goal_proven'])
        self.assertEqual(current['selected_inventory_goal']['first_satisfied_anchor'], first)
        self.assert_single_original_effects(current, original)

    def test_parent_goal_requires_advancing_original_stability_and_survives_order_retirement(self):
        sid, original, _ = self.inventory_plan(stable_samples=2)
        first = self.goal_sample(sid, TICK + 5)
        self.assertEqual(first['current_parent_goal']['current_truth'], 'true')
        self.assertFalse(first['original_goal_proven'])
        self.assertFalse(first['selected_inventory_goal']['proven_at_anchor'])
        duplicate = self.goal_sample(sid, TICK + 5)
        self.assertFalse(duplicate['original_goal_proven'])
        self.assertFalse(duplicate['selected_inventory_goal']['proven_at_anchor'])

        # The engine can retire an order; the explicitly sealed inventory goal
        # remains meaningful, without asserting this order produced the stock.
        self.peer.order_retired = True
        achieved = self.goal_sample(sid, TICK + 10)
        self.assertEqual(achieved['original_plan'], original)
        self.assertTrue(achieved['original_goal_proven'])
        selected = achieved['selected_inventory_goal']
        self.assertTrue(selected['proven_at_anchor'])
        self.assertFalse(selected['receipt_linked'])
        self.assertTrue(selected['physical_work'].startswith('Unknown'), selected)
        self.assertEqual(selected['parent_terminal_truth'], 'unknown')
        self.assertFalse(selected['original_goal_proven'])
        self.assertEqual(selected['production_attribution'], 'unknown')
        self.assertTrue(selected['preexisting_items_included'])
        self.assertEqual(selected['first_satisfied_anchor'], selected['anchor'])
        self.assertTrue(achieved['recovery']['goal_history_present'])
        self.assertIn('parent/goal.history', custody_files(self.directory))
        calls = list(self.peer.calls)
        explained = self.client.tool('explain', session_id=sid)
        self.assertTrue(explained['ok'], explained)
        self.assertEqual(explained['current_parent_goal'], achieved['current_parent_goal'])
        self.assertEqual(self.peer.calls, calls)

    def test_parent_goal_reacquires_labor_instead_of_reusing_applied_receipt(self):
        sid, original, _ = self.inventory_plan()
        achieved = self.goal_sample(sid, TICK + 5)
        self.assertTrue(achieved['original_goal_proven'])
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.peer.workforce_labor = False
        reversed_labor = self.goal_sample(sid, TICK + 10)
        self.assertEqual(reversed_labor['original_plan'], original)
        self.assertTrue(reversed_labor['frontier']['all_applied'])
        self.assertEqual(reversed_labor['current_parent_goal']['current_truth'], 'false')
        self.assertFalse(reversed_labor['original_goal_proven'])
        self.assertTrue(reversed_labor['selected_inventory_goal']['proven_at_anchor'])
        self.assertEqual(reversed_labor['selected_inventory_goal']['first_satisfied_anchor'], historical)
        self.assertEqual(reversed_labor['frontier']['entries'][0]['receipt_digest'],
                         achieved['frontier']['entries'][0]['receipt_digest'])

    def test_parent_goal_rejects_labor_drift_within_inventory_bracket(self):
        sid, _, _ = self.inventory_plan()
        achieved = self.goal_sample(sid, TICK + 5)
        self.assertTrue(achieved['original_goal_proven'])
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.peer.goal_tick(TICK + 10)
        self.peer.after_operations_read = lambda: setattr(self.peer, 'workforce_labor', False)
        start = len(self.peer.calls)
        refused = self.client.tool('wait', session_id=sid)
        self.assertFalse(refused['ok'], refused)
        self.assertFalse(refused['original_goal_proven'])
        calls = self.peer.calls[start:]
        workforce_reads = [index for index, call in enumerate(calls)
                           if call == ('dfmcp_workforce_v1_17', 'ObserveWorkforce')]
        inventory_read = calls.index(('dfmcp_operations_v1_3', 'ReadObservation'))
        self.assertEqual(len(workforce_reads), 2)
        self.assertLess(workforce_reads[0], inventory_read)
        self.assertLess(inventory_read, workforce_reads[1])
        current = self.client.tool('query', session_id=sid)
        self.assertTrue(current['ok'], current)
        self.assertIsNone(current['current_parent_goal'])
        self.assertFalse(current['original_goal_proven'])
        self.assertIsNone(current['selected_inventory_goal']['current_truth'])
        self.assertFalse(current['selected_inventory_goal']['proven_at_anchor'])
        self.assertEqual(current['selected_inventory_goal']['first_satisfied_anchor'], historical)
        self.assert_single_dispatch()

    def test_parent_goal_withdraws_proof_after_late_custody_publication_failure(self):
        sid, _, _ = self.inventory_plan()
        achieved = self.goal_sample(sid, TICK + 5)
        self.assertTrue(achieved['original_goal_proven'])
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        archive = self.directory / 'parent/original.plan'
        history = self.directory / 'parent/goal.history'
        history_bytes = history.stat().st_size
        original_bytes = archive.read_bytes()
        reads = []
        def refuse_final_publication():
            reads.append(True)
            if len(reads) == 2:
                # The selected sample has published, but the final parent
                # custody check must reject this noncanonical file mode.
                archive.chmod(0o400)
        self.peer.after_workforce_read = refuse_final_publication
        self.peer.goal_tick(TICK + 10)
        try:
            refused = self.client.tool('wait', session_id=sid)
        finally:
            archive.chmod(0o600)
            self.peer.after_workforce_read = None
        self.assertEqual(len(reads), 2)
        self.assertFalse(refused['ok'], refused)
        self.assertFalse(refused['original_goal_proven'])
        self.assertIsNone(refused['retained_reference']['current_parent_goal'])
        self.assertEqual(archive.read_bytes(), original_bytes)
        current = self.client.tool('query', session_id=sid)
        # A detected archive custody violation fences this owner. Restoring
        # permissions does not silently rehabilitate it or its last proof.
        self.assertFalse(current['ok'], current)
        self.assertFalse(current['original_goal_proven'])
        retained = current['retained_reference']
        self.assertIsNone(retained['current_parent_goal'])
        self.assertFalse(retained['selected_inventory_goal']['proven_at_anchor'])
        self.assertIsNone(retained['selected_inventory_goal']['current_truth'])
        self.assertEqual(retained['selected_inventory_goal']['first_satisfied_anchor'], historical)
        self.assertGreater(history.stat().st_size, history_bytes)
        self.assert_single_dispatch()

    def test_parent_goal_is_not_restored_from_inventory_history_after_restart(self):
        sid, original, _ = self.inventory_plan()
        achieved = self.goal_sample(sid, TICK + 5)
        self.assertTrue(achieved['original_goal_proven'])
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        recovery = achieved['recovery']
        workforce_id = achieved['native_custody']['workforce_journal_id']
        self.client.close(crash=True)
        before = custody_files(self.directory)
        calls = list(self.peer.calls)
        env = reopen_environment(self.env, recovery, workforce_id)
        self.client = stdio.Client(self.directory, {}, suffix='goal-offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertEqual(offline['original_plan'], original)
        self.assertEqual(offline['selected_inventory_goal']['first_satisfied_anchor'], historical)
        self.assertFalse(offline['selected_inventory_goal']['proven_at_anchor'])
        self.assertIsNone(offline['selected_inventory_goal']['current_truth'])
        self.assertIsNone(offline['current_parent_goal'])
        self.assertFalse(offline['original_goal_proven'])
        self.assertFalse(offline['current_state_proven'])
        self.assertEqual(self.peer.calls, calls)
        self.assertEqual(custody_files(self.directory), before)

        self.client.close()
        self.client = stdio.Client(self.directory, {}, suffix='goal-reconcile', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000)
        self.assertTrue(reopened['ok'], reopened)
        self.assertIsNone(reopened['current_parent_goal'])
        self.assertFalse(reopened['original_goal_proven'])
        fresh = self.goal_sample(reopened['agent_turn']['session_id'], TICK + 10)
        self.assertTrue(fresh['original_goal_proven'])
        self.assertEqual(fresh['original_plan'], original)
        self.assertEqual(fresh['selected_inventory_goal']['first_satisfied_anchor'], historical)

    def test_parent_goal_withdraws_current_inventory_but_preserves_first_achievement(self):
        sid, original, _ = self.inventory_plan()
        self.peer.order_retired = True
        achieved = self.goal_sample(sid, TICK + 5)
        self.assertTrue(achieved['original_goal_proven'])
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.peer.inventory_stack = 0
        consumed = self.goal_sample(sid, TICK + 10)
        self.assertEqual(consumed['original_plan'], original)
        self.assertEqual(consumed['current_parent_goal']['current_truth'], 'false')
        self.assertFalse(consumed['original_goal_proven'])
        selected = consumed['selected_inventory_goal']
        self.assertEqual(selected['current_truth'], 'false')
        self.assertFalse(selected['proven_at_anchor'])
        self.assertEqual(selected['first_satisfied_anchor'], historical)
        self.assertFalse(selected['receipt_linked'])
        self.assertTrue(selected['physical_work'].startswith('Unknown'), selected)
        self.assertEqual(selected['production_attribution'], 'unknown')

    def test_parent_source_regression_is_visible_and_fences_subsequent_native_reads(self):
        sid, _, _ = self.inventory_plan()
        achieved = self.goal_sample(sid, TICK + 5)
        self.assertTrue(achieved['original_goal_proven'])
        self.assertFalse(achieved['parent_goal_observation_fenced'])
        historical = achieved['selected_inventory_goal']['first_satisfied_anchor']
        self.peer.goal_tick(TICK + 10)
        self.peer.workforce_sequence = 3  # Applied source fence was four, in the same incarnation.
        start = len(self.peer.calls)
        refused = self.client.tool('wait', session_id=sid)
        self.assertFalse(refused['ok'], refused)
        self.assertFalse(refused['original_goal_proven'])
        retained = refused['retained_reference']
        self.assertTrue(retained['parent_goal_observation_fenced'])
        self.assertIsNone(retained['current_parent_goal'])
        self.assertGreaterEqual(retained['observation_tick_floor'], TICK + 10)
        calls = self.peer.calls[start:]
        self.assertEqual(calls.count(('dfmcp_workforce_v1_17', 'ObserveWorkforce')), 1)
        self.assertNotIn(('dfmcp_operations_v1_3', 'ReadObservation'), calls)
        current = self.client.tool('query', session_id=sid)
        self.assertTrue(current['ok'], current)
        self.assertTrue(current['parent_goal_observation_fenced'])
        self.assertIsNone(current['current_parent_goal'])
        self.assertFalse(current['original_goal_proven'])
        self.assertFalse(current['selected_inventory_goal']['proven_at_anchor'])
        self.assertEqual(current['selected_inventory_goal']['first_satisfied_anchor'], historical)
        self.assertNotIn('fortress_wait', [step['tool'] for step in current['agent_turn']['recommendations']])
        calls = list(self.peer.calls)
        self.peer.workforce_sequence = 4
        refused_again = self.client.tool('wait', session_id=sid)
        self.assertFalse(refused_again['ok'], refused_again)
        self.assertTrue(refused_again['retained_reference']['parent_goal_observation_fenced'])
        self.assertEqual(self.peer.calls, calls)
        self.assert_single_dispatch()

    def test_preplan_lifecycle_refusals_do_not_invent_an_uncertain_effect(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        calls = list(self.peer.calls)
        before = custody_files(self.directory)
        for name, args in (
            ('commit', {'paired_seal': 'a' * 64, 'confirmed': True}),
            ('plan', {'request': json.dumps({'kind': 'step', 'original_plan_digest': 'a' * 64})}),
            ('cancel', {'scope': 'step'}),
            ('wait', {}),
        ):
            failed = self.client.tool(name, session_id=sid, **args)
            self.assertFalse(failed['ok'], failed)
            self.assertIs(failed['game_mutation_dispatched'], False)
            self.assertEqual(failed['agent_turn']['active_work']['indeterminate_effects'], [])
            self.assertNotIn('fortress_wait', [r['tool'] for r in failed['agent_turn']['recommendations']])
            self.assertEqual(self.peer.calls, calls)
            self.assertEqual(custody_files(self.directory), before)
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        observed = self.client.execute_next(queried)
        self.assertTrue(observed['ok'], observed)
        self.assertTrue(observed['observation']['seal'])
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)
        self.assertFalse(any(op.startswith(('Prepare', 'Commit')) for _, op in self.peer.calls))

    def test_public_labor_then_finite_order_with_confirmation(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        self.assertEqual(opened['agent_turn']['recommendations'][0]['tool'], 'fortress_observe')
        self.assertEqual(opened['agent_turn']['recommendations'][0]['arguments']['session_id'], sid)
        observed = self.client.execute_next(opened)
        self.assertTrue(observed['ok'], observed)
        choices = observed['observation']['workforce_choices']
        self.assertTrue(choices['native_observed'])
        self.assertFalse(choices['mutation_authority'])
        self.assertFalse(choices['mutation_precondition_evidence'])
        self.assertTrue(choices['revalidation_required'])
        self.assertEqual(choices['canonical_observation_seal'], observed['observation']['seal'])
        offers = [(option['labor'], unit) for option in choices['options'] for unit in option['enable_candidates']]
        self.assertEqual(offers, [('MINE', '43')])
        labor, unit = offers[0]
        request = {'expected_observation': observed['observation']['seal'], 'units': [unit],
                   'labor': labor, 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        self.assertIsNone(planned['observation']['workforce_choices'])
        original = planned['original_plan']
        self.assertEqual(len(original['steps']), 2)
        self.assertEqual(original['steps'][1]['depends_on'], [0])
        digest = original['plan_digest']
        for index in (0, 1):
            previous = planned if index == 0 else committed
            recommendation = previous['agent_turn']['recommendations'][0]
            self.assertEqual(recommendation['tool'], 'fortress_plan')
            self.assertEqual(recommendation['arguments']['session_id'], sid)
            self.assertEqual(json.loads(recommendation['arguments']['request']), {'kind': 'step', 'original_plan_digest': digest})
            step = self.client.execute_next(previous)
            self.assertTrue(step['ok'], step)
            review = step['review']
            self.assertEqual(review['step_id'], index)
            self.assertEqual(review['original_plan_digest'], digest)
            self.assertEqual(review['idempotency_key'], original['steps'][index]['key'])
            self.assertTrue(review['confirmation_required'])
            native = review['native_review']
            self.assertTrue(native['paused'])
            self.assertEqual(native['tick'], TICK)
            if index == 0:
                self.assertEqual(review['family'], 'workforce/1.17')
                self.assertEqual(native['capture_witness'], hashlib.sha256(workforce_capture()).hexdigest())
                self.assertEqual((native['labor'], native['detail_index'], native['selected_only']), ('MINE', 0, True))
                self.assertEqual(len(native['citizens']), 1)
                citizen = native['citizens'][0]
                self.assertEqual((citizen['canonical_id'], citizen['native_id']), ('43', 42))
                self.assertTrue(citizen['eligible_observed'])
                self.assertFalse(citizen['member_observed'])
                self.assertTrue(citizen['member_intended'])
                self.assertFalse(citizen['labor_observed'])
                self.assertTrue(citizen['labor_intended'])
                self.assertEqual(review['native_plan_digest'], self.peer.workforce_plan['plan_digest'])
            else:
                self.assertEqual(review['family'], 'work-orders/1.10')
                self.assertEqual((native['recipe'], native['amount'], native['next_order_id']), ('wooden_bed', 1, 100))
                self.assertEqual(native['queue_witness'], hashlib.sha256(order_queue(False)).hexdigest())
                self.assertEqual(review['native_plan_digest'], self.peer.order_plan['digest'].hex())
                self.assertFalse(native['insertion_is_completion'])
            recommendation = step['agent_turn']['recommendations'][0]
            self.assertEqual(recommendation['tool'], 'fortress_commit')
            self.assertEqual(recommendation['arguments'], {'session_id': sid, 'paired_seal': review['paired_seal'], 'confirmed': True})
            committed = self.client.execute_next(step)
            self.assertTrue(committed['ok'], committed)
        result = committed
        self.assertTrue(result['frontier']['all_applied'])
        self.assertFalse(result['original_goal_proven'])
        self.assertTrue(self.peer.workforce_applied)
        self.assertTrue(self.peer.order_created)
        self.assertEqual([entry['phase'] for entry in result['frontier']['entries']], ['applied', 'applied'])
        self.assertEqual([entry['idempotency_key'] for entry in result['frontier']['entries']], [step['key'] for step in original['steps']])
        self.assertTrue(all(entry['receipt_digest'] and entry['dispatch_attempted'] for entry in result['frontier']['entries']))
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)
        self.assertEqual(sum(op == 'CommitOrder' for _, op in self.peer.calls), 1)
        closed = self.client.tool('cancel', session_id=sid, scope='session')
        self.assertTrue(closed['ok'], closed)
        self.assertEqual(closed['agent_turn']['recommendations'], [])
        before = custody_files(self.directory)
        self.assertTrue(before)
        self.assertTrue(all(mode == 0o600 for _, mode, _ in before.values()))
        calls = list(self.peer.calls)
        self.client.close()
        env = reopen_environment(self.env, result['recovery'], result['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='offline', env=env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertTrue(offline['frontier']['all_applied'])
        self.assertFalse(offline['original_goal_proven'])
        self.assertIsNone(offline['observation']['workforce_choices'])
        self.assertEqual(calls, self.peer.calls)
        self.assertEqual(before, custody_files(self.directory))

    def test_unconfirmed_step_never_dispatches_or_reuses_review(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        step = self.client.tool('plan', session_id=sid, request=json.dumps(
            {'kind': 'step', 'original_plan_digest': planned['original_plan']['plan_digest']}))
        self.assertTrue(step['ok'], step)
        seal = step['review']['paired_seal']
        denied = self.client.tool('commit', session_id=sid, paired_seal=seal, confirmed=False)
        self.assertFalse(denied['ok'], denied)
        stale = self.client.tool('commit', session_id=sid, paired_seal=seal, confirmed=True)
        self.assertFalse(stale['ok'], stale)
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)
        self.assertNotIn(('dfmcp_workforce_v1_17', 'CommitAssignment'), self.peer.calls)
        self.assertNotIn(('dfmcp_work_orders_v1_10', 'CommitOrder'), self.peer.calls)

    def test_missing_parent_archive_refuses_before_native_dispatch(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        (self.directory / 'parent' / 'original.plan').unlink()
        attempt = self.client.tool('plan', session_id=sid, request=json.dumps(
            {'kind': 'step', 'original_plan_digest': planned['original_plan']['plan_digest']}))
        self.assertFalse(attempt['ok'], attempt)
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)
        self.assertNotIn(('dfmcp_workforce_v1_17', 'CommitAssignment'), self.peer.calls)
        self.assertNotIn(('dfmcp_work_orders_v1_10', 'CommitOrder'), self.peer.calls)

    def test_lost_reply_reconciles_original_key_without_redispatch(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        original_key = planned['original_plan']['steps'][0]['key']
        step = self.client.tool('plan', session_id=sid, request=json.dumps(
            {'kind': 'step', 'original_plan_digest': planned['original_plan']['plan_digest']}))
        self.assertTrue(step['ok'], step)
        self.peer.drop_workforce_reply = True
        result = self.client.tool('commit', session_id=sid, paired_seal=step['review']['paired_seal'], confirmed=True)
        self.assertFalse(result['ok'], result)
        self.assertTrue(self.peer.workforce_applied)
        self.assertEqual(self.peer.workforce_plan['key'], original_key)
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)
        recovery = result['retained_reference']['recovery']
        self.assertTrue(recovery['frontier']['head'])
        workforce_id = step['native_custody']['workforce_journal_id']
        self.client.close(crash=True)
        before = custody_files(self.directory)
        self.assertTrue(before)
        self.assertTrue(all(mode == 0o600 for _, mode, _ in before.values()))
        calls = list(self.peer.calls)
        offline_env = reopen_environment(self.env, recovery, workforce_id)
        for name in ('DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN', 'DFMCP_SEMANTIC_WORKFORCE_TOKEN',
                     'DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN', 'DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN',
                     'DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN', 'DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN',
                     'DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR', 'DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION'):
            offline_env.pop(name, None)
        self.client = stdio.Client(self.directory, {}, suffix='uncertain-offline', env=offline_env)
        offline = self.client.open(mode='offline', max_wall_millis=60000)
        self.assertTrue(offline['ok'], offline)
        self.assertTrue(offline['frontier']['needs_reconciliation'])
        self.assertEqual(offline['frontier']['entries'][0]['idempotency_key'], original_key)
        self.assertFalse(offline['original_goal_proven'])
        self.assertFalse(offline['labor_control_enabled'])
        self.assertFalse(offline['production_control_enabled'])
        recommendations = offline['agent_turn']['recommendations']
        self.assertTrue(recommendations)
        self.assertNotIn('fortress_wait', [item['tool'] for item in recommendations])
        self.assertEqual(recommendations[0]['tool'], 'fortress_cancel')
        self.assertEqual(recommendations[0]['arguments'], {'session_id': offline['agent_turn']['session_id'], 'scope': 'session'})
        closed = self.client.execute_next(offline)
        self.assertTrue(closed['ok'], closed)
        self.assertEqual(calls, self.peer.calls)
        self.assertEqual(before, custody_files(self.directory))
        self.client.close()
        reconcile_env = reopen_environment(self.env, offline['recovery'], workforce_id)
        self.client = stdio.Client(self.directory, {}, suffix='uncertain-reconcile', env=reconcile_env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000)
        self.assertTrue(reopened['ok'], reopened)
        self.assertTrue(reopened['frontier']['needs_reconciliation'])
        reconciled = self.client.tool('wait', session_id=reopened['agent_turn']['session_id'])
        self.assertTrue(reconciled['ok'], reconciled)
        first = reconciled['frontier']['entries'][0]
        self.assertEqual(first['idempotency_key'], original_key)
        self.assertEqual(first['phase'], 'applied')
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)
        self.assertFalse(self.peer.order_created)

    def test_restarted_session_rejects_old_review(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        step = self.client.tool('plan', session_id=sid, request=json.dumps(
            {'kind': 'step', 'original_plan_digest': planned['original_plan']['plan_digest']}))
        self.assertTrue(step['ok'], step)
        seal = step['review']['paired_seal']
        recovery = step['recovery']
        self.client.close(crash=True)
        env = reopen_environment(self.env, recovery, step['native_custody']['workforce_journal_id'])
        self.client = stdio.Client(self.directory, {}, suffix='reopened', env=env)
        reopened = self.client.open(mode='reconcile', max_wall_millis=60000)
        self.assertTrue(reopened['ok'], reopened)
        denied = self.client.tool('commit', session_id=reopened['agent_turn']['session_id'], paired_seal=seal, confirmed=True)
        self.assertFalse(denied['ok'], denied)
        self.assertFalse(self.peer.workforce_applied)
        self.assertNotIn(('dfmcp_workforce_v1_17', 'CommitAssignment'), self.peer.calls)

    def test_replaced_native_generation_refuses_before_dispatch(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        request = {'expected_observation': observed['observation']['seal'], 'units': ['43'],
                   'labor': 'MINE', 'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        step = self.client.tool('plan', session_id=sid, request=json.dumps(
            {'kind': 'step', 'original_plan_digest': planned['original_plan']['plan_digest']}))
        self.assertTrue(step['ok'], step)
        self.peer.workforce_generation += 1
        rejected = self.client.tool('commit', session_id=sid, paired_seal=step['review']['paired_seal'], confirmed=True)
        self.assertFalse(rejected['ok'], rejected)
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)
        self.assertNotIn(('dfmcp_workforce_v1_17', 'CommitAssignment'), self.peer.calls)

    def test_unknown_operator_environment_refuses_before_native_contact(self):
        self.client.close()
        env = self.env.copy(); env['DFMCP_UNEXPECTED_COMBINED_TEST_VALUE'] = '1'
        self.client = stdio.Client(self.directory, {}, suffix='unknown-env', env=env)
        rejected = self.client.open(max_wall_millis=60000)
        self.assertFalse(rejected['ok'], rejected)
        self.assertEqual(self.peer.calls, [])

    def test_missing_citizen_credential_refuses_without_further_contact(self):
        self.client.close()
        env = self.env.copy(); del env['DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN']
        self.client = stdio.Client(self.directory, {}, suffix='missing-credential', env=env)
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        calls_after_open = list(self.peer.calls)
        self.assertTrue(all(call == ('dfmcp_work_orders_v1_10', 'Handshake') for call in calls_after_open))
        observed = self.client.tool('observe', session_id=opened['agent_turn']['session_id'])
        self.assertFalse(observed['ok'], observed)
        self.assertEqual(self.peer.calls, calls_after_open)

    def test_observation_source_drift_withdraws_choices_without_setter(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        self.assertTrue(observed['observation']['workforce_choices']['options'])
        self.peer.workforce_generation += 1
        rejected = self.client.tool('observe', session_id=sid)
        self.assertFalse(rejected['ok'], rejected)
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertIsNone(queried['observation'])
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)
        self.assertNotIn(('dfmcp_workforce_v1_17', 'PrepareAssignment'), self.peer.calls)
        self.assertNotIn(('dfmcp_workforce_v1_17', 'CommitAssignment'), self.peer.calls)
        self.assertNotIn(('dfmcp_work_orders_v1_10', 'CommitOrder'), self.peer.calls)

    def test_cross_source_tick_floor_survives_failed_observation(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.tool('observe', session_id=sid)
        self.assertTrue(observed['ok'], observed)
        self.assertTrue(observed['observation']['workforce_choices']['options'])
        self.peer.workforce_tick = TICK + 1
        mismatched = self.client.tool('observe', session_id=sid)
        self.assertFalse(mismatched['ok'], mismatched)
        self.assertEqual(mismatched['retained_reference']['observation_tick_floor'], TICK + 1)
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertEqual(queried['observation_tick_floor'], TICK + 1)
        self.assertIsNone(queried['observation'])
        self.peer.workforce_tick = TICK
        workforce_calls = [call for call in self.peer.calls if call[0] == 'dfmcp_workforce_v1_17']
        rolled_back = self.client.tool('observe', session_id=sid)
        self.assertFalse(rolled_back['ok'], rolled_back)
        self.assertEqual(rolled_back['retained_reference']['observation_tick_floor'], TICK + 1)
        self.assertEqual(workforce_calls, [call for call in self.peer.calls if call[0] == 'dfmcp_workforce_v1_17'])
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertEqual(queried['observation_tick_floor'], TICK + 1)
        self.assertIsNone(queried['observation'])
        for method in ('PrepareAssignment', 'CommitAssignment'):
            self.assertNotIn(('dfmcp_workforce_v1_17', method), self.peer.calls)
        self.assertNotIn(('dfmcp_work_orders_v1_10', 'CommitOrder'), self.peer.calls)
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)


    def test_selected_citizen_and_later_option_execute_original_plan(self):
        self.peer.citizen_ids = list(range(10, 43))
        self.peer.option_count = 10
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        first = self.client.execute_next(opened)
        self.assertTrue(first['ok'], first)
        choices = first['observation']['workforce_choices']
        self.assertEqual(choices['coverage']['canonical_citizens'], 33)
        self.assertEqual(choices['coverage']['selected_citizens'], 32)
        self.assertEqual(choices['coverage']['omitted_citizens'], 1)
        self.assertNotIn('43', [unit['entity_id'] for unit in choices['selected_citizens']])
        self.assertIn('43', [unit['entity_id'] for unit in first['observation']['citizens']])
        self.assertEqual(self.peer.workforce_selections[-1], list(range(10, 42)))

        selected = self.client.tool('observe', session_id=sid, request=json.dumps(
            {'units': ['43', '42'], 'option_offset': 8}))
        self.assertTrue(selected['ok'], selected)
        choices = selected['observation']['workforce_choices']
        self.assertEqual(self.peer.workforce_selections[-1], [41, 42])
        self.assertEqual([unit['entity_id'] for unit in choices['selected_citizens']], ['42', '43'])
        self.assertEqual([option['labor'] for option in choices['options']], ['LABOR_8', 'LABOR_9'])
        self.assertEqual(choices['coverage']['available_options'], 10)
        self.assertEqual(choices['coverage']['option_offset'], 8)
        self.assertEqual(choices['coverage']['omitted_options'], 8)
        self.assertIsNone(choices['coverage']['next_option_offset'])
        self.assertNotEqual(choices['witness'], first['observation']['workforce_choices']['witness'])
        self.assertEqual(choices['canonical_observation_seal'], selected['observation']['seal'])
        self.assertFalse(choices['mutation_authority'])
        self.assertFalse(choices['mutation_precondition_evidence'])
        self.assertTrue(choices['revalidation_required'])

        option = choices['options'][0]
        request = {'expected_observation': selected['observation']['seal'],
                   'units': option['enable_candidates'], 'labor': option['labor'],
                   'enabled': True, 'recipe': 'wooden_bed', 'amount': 1,
                   'deadline_tick': 300, 'poll_interval_ticks': 5, 'stable_samples': 2}
        planned = self.client.tool('plan', session_id=sid, request=json.dumps(request))
        self.assertTrue(planned['ok'], planned)
        original = planned['original_plan']
        self.assertEqual(original['steps'][0]['action']['units'], ['42', '43'])
        self.assertEqual(original['steps'][0]['action']['labor'], 'LABOR_8')
        calls = list(self.peer.calls)
        denied = self.client.tool('observe', session_id=sid, request=json.dumps({'units': ['43']}))
        self.assertFalse(denied['ok'], denied)
        self.assertEqual(self.peer.calls, calls)
        current = self.client.tool('observe', session_id=sid)
        self.assertTrue(current['ok'], current)
        self.assertEqual(current['original_plan'], original)
        for index in (0, 1):
            prepared = self.client.execute_next(current)
            self.assertTrue(prepared['ok'], prepared)
            self.assertEqual(prepared['review']['step_id'], index)
            self.assertEqual(prepared['review']['idempotency_key'], original['steps'][index]['key'])
            current = self.client.execute_next(prepared)
            self.assertTrue(current['ok'], current)
        self.assertTrue(current['frontier']['all_applied'])
        self.assertFalse(current['original_goal_proven'])
        self.assertFalse(current['current_state_proven'])
        self.assertEqual(sum(op == 'CommitAssignment' for _, op in self.peer.calls), 1)
        self.assertEqual(sum(op == 'CommitOrder' for _, op in self.peer.calls), 1)

    def test_closed_observation_selectors_withdraw_choices_without_native_writes(self):
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        observed = self.client.execute_next(opened)
        self.assertTrue(observed['ok'], observed)
        calls = list(self.peer.calls)
        invalid = [
            '{"units":[]}', '{"units":["43","43"]}', '{"units":["043"]}',
            '{"units":["0"]}', '{"units":[43]}', '{"units":["+43"]}',
            '{"units":["43"],"native_id":42}', '{"option_offset":-1}',
            '{"option_offset":129}', '{"option_offset":0,"option_offset":0}',
            json.dumps({'units': [str(n) for n in range(1, 34)]}), ' ' * 4097,
        ]
        for raw in invalid:
            rejected = self.client.tool('observe', session_id=sid, request=raw)
            self.assertFalse(rejected['ok'], rejected)
            self.assertIs(rejected['game_mutation_dispatched'], False)
            self.assertEqual(self.peer.calls, calls)
            self.assertEqual(rejected['agent_turn']['active_work']['indeterminate_effects'], [])
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertIsNone(queried['observation']['workforce_choices'])
        workforce_calls = [call for call in self.peer.calls if call[0] == 'dfmcp_workforce_v1_17']
        unknown = self.client.tool('observe', session_id=sid, request='{"units":["999"]}')
        self.assertFalse(unknown['ok'], unknown)
        self.assertEqual(workforce_calls, [call for call in self.peer.calls if call[0] == 'dfmcp_workforce_v1_17'])
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertIsNone(queried['observation'])
        self.assertFalse(any(op.startswith(('Prepare', 'Commit')) for _, op in self.peer.calls))
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)

    def test_option_pages_offer_fresh_reads_and_reject_out_of_range(self):
        self.peer.option_count = 10
        opened = self.client.open(max_wall_millis=60000)
        self.assertTrue(opened['ok'], opened)
        sid = opened['agent_turn']['session_id']
        first = self.client.execute_next(opened)
        self.assertTrue(first['ok'], first)
        choices = first['observation']['workforce_choices']
        self.assertEqual([option['labor'] for option in choices['options']],
                         [f'LABOR_{i}' for i in range(8)])
        self.assertEqual(choices['next_observation_request'], {'units': ['43'], 'option_offset': 8})
        recommendations = [r for r in first['agent_turn']['recommendations'] if r['tool'] == 'fortress_observe']
        self.assertEqual(len(recommendations), 1)
        self.assertEqual(json.loads(recommendations[0]['arguments']['request']),
                         choices['next_observation_request'])
        second = self.client.tool('observe', **recommendations[0]['arguments'])
        self.assertTrue(second['ok'], second)
        self.assertEqual([option['labor'] for option in second['observation']['workforce_choices']['options']],
                         ['LABOR_8', 'LABOR_9'])
        self.assertNotEqual(second['observation']['seal'], first['observation']['seal'])
        self.assertIsNone(second['observation']['workforce_choices']['next_observation_request'])
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertEqual(queried['observation']['workforce_choices'], second['observation']['workforce_choices'])
        exhausted = self.client.tool('observe', session_id=sid,
                                     request='{"units":["43"],"option_offset":10}')
        self.assertTrue(exhausted['ok'], exhausted)
        choices = exhausted['observation']['workforce_choices']
        self.assertEqual(choices['options'], [])
        self.assertEqual(choices['coverage']['available_options'], 10)
        self.assertEqual(choices['coverage']['omitted_options'], 10)
        self.assertIsNone(choices['next_observation_request'])
        rejected = self.client.tool('observe', session_id=sid,
                                    request='{"units":["43"],"option_offset":11}')
        self.assertFalse(rejected['ok'], rejected)
        queried = self.client.tool('query', session_id=sid)
        self.assertTrue(queried['ok'], queried)
        self.assertIsNone(queried['observation'])
        self.assertFalse(any(op.startswith(('Prepare', 'Commit')) for _, op in self.peer.calls))
        self.assertFalse(self.peer.workforce_applied)
        self.assertFalse(self.peer.order_created)

def main():
    global LOGS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--logs', type=Path, required=True)
    args = parser.parse_args()
    stdio.BINARY = args.binary.resolve(strict=True)
    stdio.SERVER_ARGS = ['serve-gameplay-dev', 'labor-order']
    LOGS = args.logs
    LOGS.mkdir(mode=0o700)
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(CombinedProduction))
    print(json.dumps({'schema': 'dfmcp.combined-production-stdio/1', 'tests': result.testsRun,
        'failed': len(result.failures), 'errors': len(result.errors), 'skipped': len(result.skipped),
        'actual_modern_stdio': True, 'loopback_peers_are_doubles': True, 'live_game': False}))
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 40 and not result.skipped else 1)


if __name__ == '__main__': main()
