"""Actual final reader/receipt/map/operations codecs over joined synthetic TCP.

Batch custody is an explicit port; this is not real private-batch or DFHack evidence.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import replace
import hashlib
import socket
import struct
import threading
import unittest
from unittest.mock import patch

import construction_monitor_rpc as wire
from construction_receipt import Manifest
from build_placement_wire import Capture, Tile, Item, Selection, Plan, Record, Insertion
from furniture_allocation import Request, Slot, Candidate
from furniture_plan import canonical, progress
import route_furniture_allocation as routes
from route_furniture_review import Source, create
import route_furniture_final as final


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def text(value):
    value = value.encode()
    return struct.pack('>H', len(value)) + value


def terrain(n=2, tick=10, *, folder='region1', site=2, occupied=True):
    raw = b'DFMM1500' + struct.pack('>IIBI', 0, tick, 1, site) + text(folder)
    raw += struct.pack('>10I', 40, 6, 1, 1, 1, 0, n+2, 3, 1, (n+2)*3)
    for index in range((n+2)*3):
        x, y = index % (n+2) + 1, index // (n+2) + 1
        building = int(occupied and y == 2 and 2 <= x < n+2)
        raw += b'\2' + struct.pack('>IBBBBBBBIHH', 1, 3, 0, 0, 0, 0, building, 0, 1, 10015, 10015)
    return raw


def operations(n=2, tick=10, *, pending=(), missing=(), removal=(), other_job=(),
               extra_items=0, folder='region1', site=2, paused=True):
    pending, missing, removal, other_job = map(set, (pending, missing, removal, other_job))
    jobs, attachments, buildings, items = [], [], [], []
    for i in range(n):
        kind = i % 3 + 1
        target = (i+2, 2, 0)
        native_kind = ('', 'BED', 'CHAIR', 'TABLE')[kind]
        if i not in missing:
            buildings.append(struct.pack('>Ii', 200+i, 100+kind)
                + text(('', 'Bed', 'Chair', 'Table')[kind])
                + struct.pack('>7i', i+2, 2, i+2, 2, 0, 0 if i in pending else 3, 3))
        holder = b'\1' + struct.pack('>I', 200+i) if i not in pending and i not in missing else b'\0'
        items.append(struct.pack('>Ii', 300+i, 30+kind) + text(native_kind)
            + struct.pack('>iiiIiiiI', -1, 419, -1, 1, *target, 192 if i in pending else 256)
            + b'\0' + holder)
        if i in pending | removal | other_job:
            name, enum = ('DestroyBuilding', 2) if i in removal else ('OtherWork', 3) if i in other_job else ('ConstructBuilding', 1)
            jobs.append(struct.pack('>Ii', 600+i if i in other_job else 100+i, enum) + text(name) + text('') + b'\0\0'
                + struct.pack('>3i', *target) + b'\0\1' + struct.pack('>I', 200+i)
                + struct.pack('>iII', -1, 1 if i in pending else 0, 0))
            if i in pending:
                attachments.append(struct.pack('>4i', 100+i, 300+i, 0, -1))
    for i in range(extra_items):
        items.append(struct.pack('>Ii', 1000+i, 80) + text('BOULDER')
            + struct.pack('>iiiIiiiI', -1, 0, 0, 1, 1, 1, 0, 64) + b'\0\0')
    jobs_raw = (b'DFMJ1200' + struct.pack('>IIBiI', 0, tick, paused, site, 600+n if other_job else 100+n)
                + text(folder) + struct.pack('>I', len(jobs)) + b''.join(jobs))
    return (b'DFMO1400' + struct.pack('>I', len(jobs_raw)) + jobs_raw
        + struct.pack('>III', 200+n, 1000+extra_items, len(buildings)) + b''.join(buildings)
        + struct.pack('>I', len(items)) + b''.join(items)
        + struct.pack('>I', len(attachments)) + b''.join(attachments))


class Batch:
    """Explicit custody port; real immutable reviews, plans and receipt codecs."""
    def __init__(self, address=('127.0.0.1', 5000), n=2):
        self.id, self.address, self.writable = 'a'*64, address, False
        self.budget = wire.Budget(60000)
        self.native = Manifest(11, 'df', 'dfhack')
        slots = tuple(Slot(f's{i:02}', ('bed', 'chair', 'table')[i % 3], (i+2, 2, 0)) for i in range(n))
        self.request = Request('region1', 2, slots)
        items = tuple(Candidate(300+i, s.kind, s.target, (419, -1), -1) for i, s in enumerate(slots))
        raw = terrain(n, occupied=False)
        self.source = Source(f'{address[0]}:{address[1]}', 'df', 'dfhack', 7, 9, sha(raw), 'b'*64,
                             10, (40, 6, 1), (1, 1, 0), (n+2, 3, 1), (100, 200, 1000))
        allocation = routes.allocate_routes(self.request, items, raw)
        self.review = create(self.request, allocation, self.source, {300+i:31+i%3 for i in range(n)})
        self.plan = self.review.unpack()[3]
        tile, receipts = Tile(2, 1, 3), []
        for i, step in enumerate(self.plan.ordered):
            kind = i % 3 + 1
            item = Item(presence=2, pos=step.target, kind=kind, native_type=30+kind,
                        subtype=-1, material=419, material_index=-1, on_ground=True, ground=tile)
            before = Capture(11, i, 10, 2, (40, 6, 1), 200+i, 100+i, i, 'region1',
                             True, True, True, Selection(kind, step.item, *step.target), (tile,)*9, item)
            plan = Plan('frt-'+self.id+'-'+step.name, before)
            insertion = Insertion(200+i, 100+i, step.item, kind, step.target, 419, -1,
                                  0, 3, True, True, True, False)
            receipts.append(Record(plan, 'placed', 'none', before.expected_after(), insertion).raw)
        self.receipts = tuple(receipts)
        self.entries = [None] * n
        self.status_override = {}
        self.stopped = False
        self.bad = False

    def check(self):
        self.budget.remaining()
        if self.bad:
            raise ValueError('lost original custody')

    def audit(self):
        self.check()
        state = progress(self.plan, {s.name:'placed' for s in self.plan.steps})
        for row in state['steps']:
            row['registered'] = True
        state.update(batch_id=self.id, review_digest=self.review.digest, inventory_verified=True,
                     inventory_digest=sha(b''.join(self.receipts)), stopped=self.stopped, advance_allowed=False)
        state.update(self.status_override)
        return state

    def placed_prefix(self):
        self.check()
        return self.receipts


class Authority:
    def __init__(self, address):
        self.address = address
        self.token, self.operations_token, self.map_token = b'b'*32, b'o'*32, b'm'*32
        self.revoked = False
        self.visits = 0
        self.at = None

    def guard(self, operation='QueryPlacement'):
        if operation not in ('QueryPlacement', 'ReadPlacement', 'Handshake'):
            raise ValueError('no mutations')
        self.visits += 1
        if self.revoked or self.visits == self.at:
            raise ValueError('authority revoked')


def varint(number):
    out = bytearray()
    while number > 127:
        out.append((number & 127) | 128)
        number >>= 7
    return bytes(out) + bytes([number])


def protobuf(fields):
    out = b''
    for key, value in sorted(fields.items()):
        out += varint(key*8 + (2 if isinstance(value, bytes) else 0))
        out += varint(len(value)) + value if isinstance(value, bytes) else varint(value)
    return out


def read_exact(connection, count):
    data = b''
    while len(data) < count:
        chunk = connection.recv(count-len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


class Peer:
    def __init__(self, n=2, **capture_options):
        self.socket = socket.socket()
        self.socket.bind(('127.0.0.1', 0))
        self.socket.listen(1)
        self.socket.settimeout(3)
        self.address = self.socket.getsockname()
        self.batch = Batch(self.address, n)
        self.authority = Authority(self.address)
        self.ops = operations(n, **capture_options)
        self.map = terrain(n, capture_options.get('tick', 10))
        self.calls, self.bindings, self.failures = [], [], []
        self.mutate = lambda family, name, fields, reply, ordinal: reply
        self.thread = None

    def __enter__(self):
        self.thread = threading.Thread(target=self.serve)
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.thread.join(5)
        self.socket.close()
        if self.thread.is_alive():
            raise AssertionError('unjoined final-reader peer')
        if self.failures:
            raise self.failures[0]

    def serve(self):
        methods = {}
        try:
            with self.socket.accept()[0] as connection:
                connection.settimeout(3)
                assert read_exact(connection, 12) == b'DFHack?\n'+struct.pack('<i', 1)
                connection.sendall(b'DFHack!\n'+struct.pack('<i', 1))
                while True:
                    method, size = struct.unpack('<h2xi', read_exact(connection, 8))
                    assert 0 <= size <= 2048
                    fields = wire.decode(read_exact(connection, size))
                    if method == 0:
                        plugin = fields[4].decode()
                        family, minor = {'dfmcp_build_v1_19':('build',19),
                                         'dfmcp_operations_v1_4':('operations',4),
                                         'dfmcp_map_v1_5':('map',5)}[plugin]
                        name = fields[1].decode()
                        package = f'dfmcp.{"run" if family == "clock" else family}.v1_{minor}'
                        assert fields[2] == (package+'.Request').encode()
                        assert fields[3] == (package+'.Reply').encode()
                        assert (family,name) in tuple(wire.BINDINGS) + tuple(('map',x) for x in final.MAP_METHODS)
                        self.bindings.append((family,name))
                        identity = len(methods)+2
                        methods[identity] = family,name,minor
                        reply = {1:identity}
                    else:
                        family,name,minor = methods[method]
                        assert fields[1] == {'build':b'b'*32,'operations':b'o'*32,'map':b'm'*32}[family]
                        assert fields[3] == 1 and fields[4] == minor
                        self.calls.append((family,name))
                        reply = {1:1,2:0,3:fields[2],4:1,5:minor,
                                 6:{'build':11,'operations':9,'map':7}[family],7:b'df',8:b'dfhack'}
                        if family == 'build':
                            reply.update({12:0,13:len(self.batch.receipts)})
                            if name == 'QueryPlacement':
                                found = [Record.decode(r) for r in self.batch.receipts if Record.decode(r).plan.key.encode() == fields[10]]
                                assert len(found) == 1 and found[0].plan.digest == fields[12]
                                reply[10] = found[0].raw
                        elif family == 'operations' and name == 'ReadObservation':
                            token = b'x'*16
                            if fields[12] == 1:
                                assert fields[9] == token and fields[10] == 0
                                reply[10] = token
                            else:
                                offset = fields[10]
                                assert fields[9] == (b'' if offset == 0 else token)
                                page = self.ops[offset:offset+65536]
                                reply.update({9:page,10:token,11:offset,12:len(self.ops),
                                              13:hashlib.sha256(self.ops).digest(),14:int(offset+len(page)==len(self.ops))})
                        elif family == 'map':
                            assert tuple(fields[i] for i in range(5,11)) == self.batch.source.origin+self.batch.source.size
                            if name == 'ReadObservation':
                                reply[9] = self.map
                        reply = self.mutate(family,name,fields,reply,len(self.calls))
                    payload = protobuf(reply)
                    connection.sendall(struct.pack('<h2xi', -1, len(payload))+payload)
        except (EOFError, BrokenPipeError, ConnectionResetError):
            pass
        except BaseException as error:
            self.failures.append(error)


class FinalTests(unittest.TestCase):
    def test_pending_after_last_placement_has_complete_original_report(self):
        with Peer(pending=(1,)) as peer:
            state, result = final.read_frontier(peer.batch, peer.authority)
        self.assertEqual(state['status'], 'all_placed')
        self.assertIsNone(state['next_step'])
        self.assertEqual(result['status'], 'construction_pending')
        self.assertEqual(result['construction_prefix']['selected_count'], 2)
        self.assertEqual(result['construction_prefix']['counts']['quiescent'], 1)
        self.assertFalse(state['advance_allowed'])
        self.assertIsNone(state['confirm_review'])
        self.assertEqual(peer.calls.count(('build','QueryPlacement')), 4)
        operations_at = [i for i,x in enumerate(peer.calls) if x == ('operations','ReadObservation')]
        receipt_at = [i for i,x in enumerate(peer.calls) if x == ('build','QueryPlacement')]
        self.assertLess(receipt_at[1], operations_at[0])
        self.assertGreater(receipt_at[2], operations_at[-1])
        self.assertEqual(len(peer.bindings), 6)

    def test_quiescence_is_current_sample_not_goal_or_placement_permission(self):
        with Peer() as peer:
            _, out = final.read_frontier(peer.batch, peer.authority)
        self.assertEqual(out['status'], 'construction_quiescent')
        self.assertTrue(out['all_original_furnishings_quiescent_now'])
        for field in ('construction_completion_proven','continuous_stability_proven',
                      'placement_authorized','production_admitted'):
            self.assertFalse(out[field])

    def test_all_32_receipts_and_multipage_roster_are_checked(self):
        with Peer(32, pending=(31,), extra_items=2000) as peer:
            before = peer.batch.budget.calls
            state, out = final.read_frontier(peer.batch, peer.authority)
            used = before-peer.batch.budget.calls
        self.assertGreater(len(peer.ops), 65536)
        self.assertEqual(peer.calls.count(('build','QueryPlacement')), 64)
        self.assertEqual(out['construction_prefix']['selected_count'], 32)
        self.assertEqual(out['construction_prefix']['counts']['quiescent'], 31)
        self.assertEqual(used, len(peer.calls)+len(peer.bindings))
        self.assertLess(len(canonical(state)), 256*1024)

    def test_missing_removal_and_other_work_do_not_masquerade_as_complete(self):
        for options, status in (({'missing':(0,)},'construction_unestablished'),
                                ({'removal':(0,)},'construction_blocked'),
                                ({'other_job':(0,)},'construction_pending')):
            with self.subTest(options=options), Peer(**options) as peer:
                _, out = final.read_frontier(peer.batch, peer.authority)
                self.assertEqual(out['status'], status)
                self.assertFalse(out['all_original_furnishings_quiescent_now'])

    def test_incomplete_stopped_unregistered_and_uncertain_custody_refuse_before_contact(self):
        for field,value in (('status','ready'),('next_step','s01'),('pending_step','s00'),
                            ('stopped',True),('placed',1),('inventory_verified',False)):
            with self.subTest(field=field):
                batch=Batch(); batch.status_override[field]=value
                with patch.object(final,'Client') as client, self.assertRaises(ValueError):
                    final.read_frontier(batch,Authority(batch.address))
                client.assert_not_called()
        batch=Batch(); batch.status_override['steps']=[{'name':'s00','phase':'placed','registered':True}]
        with patch.object(final,'Client') as client, self.assertRaises(ValueError):
            final.read_frontier(batch,Authority(batch.address))
        client.assert_not_called()

    def test_original_receipt_order_key_and_material_cannot_change(self):
        for which in ('reverse','foreign','material','missing'):
            batch=Batch()
            receipts=list(batch.receipts)
            if which=='reverse': receipts.reverse()
            elif which=='missing': receipts.pop()
            else:
                old=Record.decode(receipts[0]); before=old.plan.before
                if which=='foreign':
                    plan=Plan('other-key',before); insertion=old.insertion
                else:
                    before=replace(before,item=replace(before.item,material=420))
                    plan=Plan(old.plan.key,before); insertion=replace(old.insertion,material=420)
                receipts[0]=Record(plan,'placed','none',before.expected_after(),insertion).raw
            batch.receipts=tuple(receipts)
            with self.subTest(which=which), patch.object(final,'Client') as client, self.assertRaises(ValueError):
                final.read_frontier(batch,Authority(batch.address))
            client.assert_not_called()

    def test_changed_native_generation_and_software_refuse(self):
        for family, field,value in (('map',6,8),('build',6,12),('operations',6,10),('map',7,b'other')):
            with self.subTest(family=family,field=field), Peer() as peer:
                peer.mutate=lambda f,n,q,r,k: {**r,field:value} if f==family else r
                with self.assertRaises(ValueError): final.read_frontier(peer.batch,peer.authority)

    def test_second_receipt_boundary_and_release_are_mandatory(self):
        for fault in ('last_receipt','release','map_change','nonce','unresolved'):
            with self.subTest(fault=fault), Peer() as peer:
                seen={'receipts':0,'maps':0}
                def mutate(f,n,q,r,k):
                    if n=='QueryPlacement':
                        seen['receipts']+=1
                        if fault=='last_receipt' and seen['receipts']==4: r[10]=r[10][:-1]
                    if f=='operations' and q.get(12)==1 and fault=='release': r[10]=b'y'*16
                    if f=='map' and n=='ReadObservation':
                        seen['maps']+=1
                        if fault=='map_change' and seen['maps']==2: r[9]=terrain(tick=11)
                    if fault=='nonce': r[3]=b'z'*32
                    if fault=='unresolved' and f=='build': r[12]=1
                    return r
                peer.mutate=mutate
                with self.assertRaises(ValueError): final.read_frontier(peer.batch,peer.authority)

    def test_wrong_world_tick_or_region_and_trailing_bytes_refuse(self):
        for fault in ('world','tick','region','ops_tail','map_tail','ops_truncated'):
            with self.subTest(fault=fault), Peer() as peer:
                if fault=='world': peer.ops=operations(folder='other')
                elif fault=='tick': peer.ops=operations(tick=11)
                elif fault=='region': peer.map=terrain(3)
                elif fault=='ops_tail': peer.ops+=b'\0'
                elif fault=='map_tail': peer.map+=b'\0'
                else: peer.ops=peer.ops[:-1]
                with self.assertRaises(ValueError): final.read_frontier(peer.batch,peer.authority)

    def test_custody_change_during_capture_cannot_publish(self):
        with Peer() as peer:
            def change(f,n,q,r,k):
                if f=='operations' and q.get(12)==1: peer.batch.status_override['inventory_digest']='c'*64
                return r
            peer.mutate=change
            with self.assertRaises(ValueError): final.read_frontier(peer.batch,peer.authority)

    def test_all_actual_network_costs_and_fixed_analysis_reservation_are_shared(self):
        with Peer() as peer:
            original=peer.batch.budget; before=original.work_steps
            final.read_frontier(peer.batch,peer.authority)
            self.assertIs(peer.batch.budget,original)
            self.assertGreater(before-original.work_steps,final.ANALYSIS_WORK)
            self.assertLess(original.network_bytes,20*1024*1024-len(peer.ops))
        batch=Batch(); batch.budget.work_steps=final.ANALYSIS_WORK
        with patch.object(final,'Client') as client, self.assertRaises(ValueError):
            final.read_frontier(batch,Authority(batch.address))
        client.assert_not_called()

    def test_revocation_during_receipt_read_closes_without_publication(self):
        with Peer() as peer:
            def revoke(f,n,q,r,k):
                if n=='QueryPlacement': peer.authority.revoked=True
                return r
            peer.mutate=revoke
            with self.assertRaises(ValueError): final.read_frontier(peer.batch,peer.authority)
        self.assertEqual(peer.calls.count(('build','QueryPlacement')),1)

    def test_closed_before_projection_and_final_output_bound(self):
        with Peer() as peer:
            observed=[]
            original_capture=final.Client.capture_once
            def capture(client):
                result=original_capture(client)
                observed.append(client)
                return result
            original_decode=final.decode_operations
            def decode(raw,guard):
                self.assertTrue(observed[-1].closed)
                return original_decode(raw,guard)
            with patch.object(final.Client,'capture_once',capture), patch.object(final,'decode_operations',decode):
                final.read_frontier(peer.batch,peer.authority)
        with Peer() as peer, patch.object(final,'MAX_OUTPUT',1):
            with self.assertRaises(ValueError): final.read_frontier(peer.batch,peer.authority)

    def test_evidence_digest_binds_all_original_rows_and_source(self):
        with Peer() as peer:
            _, out=final.read_frontier(peer.batch,peer.authority)
        unsigned={k:v for k,v in out.items() if k!='evidence_digest'}
        expected=sha(b'dfmcp-final-construction-frontier/1\0'+canonical(unsigned))
        self.assertEqual(out['evidence_digest'],expected)
        for field in ('construction_prefix','source','inventory_digest','placement_manifest'):
            changed=dict(unsigned); changed[field]=None
            self.assertNotEqual(sha(b'dfmcp-final-construction-frontier/1\0'+canonical(changed)),expected)


if __name__=='__main__':
    unittest.main()
