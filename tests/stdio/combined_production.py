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
spec = importlib.util.spec_from_file_location('workshop_stdio', ROOT / 'tests/stdio/workshop.py')
stdio = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stdio)
number, blob, fields, receive = stdio.number, stdio.blob, stdio.fields, stdio.receive

DF = b'0.51.11'
DFH = b'0.51.11-r1'
GEN = 701
WGEN = 7
OGEN = 53
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


def order_queue(created=False):
    return (b'DFMWO010' + struct.pack('>QQQII', OGEN, 11 + created, TICK, 100 + created, SITE)
            + b'\x01' + text(b'region1') + struct.pack('>I', int(created))
            + (struct.pack('>I', 100) if created else b''))


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
        self.workforce_tick = TICK
        self.order_created = False
        self.drop_workforce_reply = False
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
        if plugin == 'dfmcp_workforce_v1_17':
            response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 17)
                        + number(6, self.workforce_generation) + blob(7, DF) + blob(8, DFH))
            if op == 'ObserveWorkforce':
                ids = req[11]
                assert ids == sorted(set(ids)) and 1 <= len(ids) <= 32
                assert all(ident in self.citizen_ids for ident in ids)
                self.workforce_selections.append(ids)
                capture = bytearray(workforce_capture(ids, self.option_count))
                capture[8:16] = self.workforce_generation.to_bytes(8, 'big')
                capture[24:32] = self.workforce_tick.to_bytes(8, 'big')
                response += blob(9, bytes(capture))
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
        assert plugin == 'dfmcp_work_orders_v1_10'
        response = (number(1, 1) + number(2, 0) + blob(3, req[2]) + number(4, 1) + number(5, 10)
                    + number(6, OGEN) + blob(7, DF) + blob(8, DFH))
        if op == 'ReadOrders': response += blob(9, order_queue(self.order_created))
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
            response += blob(10, order_effect(self.order_plan, True))
        if op == 'QueryOrder' and self.order_plan:
            response += blob(10, order_effect(self.order_plan, self.order_created))
        return response

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
    env['DFMCP_COMBINED_PRODUCTION_CONFIG'] = json.dumps(parent)
    for name in ('DFMCP_SEMANTIC_WORKFORCE_CONFIG', 'DFMCP_SEMANTIC_WORK_ORDER_CONFIG'):
        config = json.loads(env[name]); config['initialize'] = False
        if name == 'DFMCP_SEMANTIC_WORKFORCE_CONFIG': config['journal_id'] = workforce_id
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
    raise SystemExit(0 if result.wasSuccessful() and result.testsRun == 14 and not result.skipped else 1)


if __name__ == '__main__': main()
