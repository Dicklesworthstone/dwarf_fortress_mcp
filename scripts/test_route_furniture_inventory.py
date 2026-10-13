"""Actual source/protocol/CLI tests with an independent, joined loopback peer.

The peer is NOT DFHack. All response encoders here are independent of production
framing/roster serializers. No native SDK, live fortress or admission is implied.
"""
from __future__ import annotations

import contextlib
import hashlib
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

import allocate_route_furniture as cli
import construction_monitor_rpc as wire
from construction_receipt import decode_operations
from furniture_plan import canonical, FurniturePlan
import route_furniture_inventory as live
from route_furniture_allocation import MAX_WORK
from test_route_furniture_allocation import capture, item, request
from furniture_allocation import Slot


def string(value):
    raw = value.encode('utf-8')
    return struct.pack('>H', len(raw)) + raw


def roster(items=None, *, tick=3, paused=True, site=2, folder='region1',
           jobs=(), buildings=(), attachments=()):
    """Independent complete DFMO1400/DFMJ1200 wire fixture."""
    if items is None:
        items = [dict(id=1, position=(1, 2, 5)), dict(id=2, position=(4, 4, 5))]
    jb = bytearray(b'DFMJ1200' + struct.pack('>IIBiI', tick // 403200, tick % 403200,
                                          paused, site, 100000) + string(folder))
    jb += struct.pack('>I', len(jobs))
    for job in jobs:
        jid = job['id']
        linked = [row for row in attachments if row[0] == jid]
        jb += struct.pack('>Ii', jid, 0) + string('ConstructBuilding') + string('')
        jb += b'\0\0' + struct.pack('>iii', 1, 1, 5) + b'\0\0'
        jb += struct.pack('>iII', -1, len(linked), 0)
    raw = bytearray(b'DFMO1400' + struct.pack('>I', len(jb)) + jb)
    raw += struct.pack('>III', 100000, 100000, len(buildings))
    for building in buildings:
        raw += struct.pack('>Ii', building['id'], 0) + string('Bed')
        raw += struct.pack('>7i', 1, 1, 1, 1, 5, 0, 1)
    raw += struct.pack('>I', len(items))
    for value in items:
        kind = value.get('kind', 'BED')
        native = value.get('native_type', {'BED':0, 'CHAIR':1, 'TABLE':2, 'OTHER':3}[kind])
        raw += struct.pack('>Ii', value['id'], native) + string(kind)
        raw += struct.pack('>iiiIiiiI', value.get('subtype',-1), value.get('material',419),
                           value.get('material_index',-1), value.get('stack',1),
                           *value.get('position',(1,1,5)), value.get('flags',64))
        for key in ('container','holder'):
            raw += bytes([key in value])
            if key in value: raw += struct.pack('>I',value[key])
    raw += struct.pack('>I',len(attachments))
    for row in attachments: raw += struct.pack('>4i',*row)
    return bytes(raw)


def varint(value):
    out=bytearray()
    while value >= 128:
        out.append((value & 127) | 128); value >>= 7
    return bytes(out + bytes([value]))


def protobuf(fields):
    raw=bytearray()
    for key,value in sorted(fields.items()):
        if type(value) is bytes:
            raw += varint(key*8+2)+varint(len(value))+value
        else:
            raw += varint(key*8)+varint(value)
    return bytes(raw)


def request_fields(raw):
    at=0
    def number():
        nonlocal at
        n=0; shift=0
        while True:
            b=raw[at]; at+=1; n|=(b&127)<<shift
            if b<128: return n
            shift+=7
    out={}
    while at<len(raw):
        tag=number(); key,kind=tag>>3,tag&7
        assert key not in out
        if kind==0: out[key]=number()
        else:
            assert kind==2
            length=number(); out[key]=raw[at:at+length]; at+=length
    assert at==len(raw)
    return out


class Peer:
    """One finite TCP connection. No production worker or timer is introduced."""
    def __init__(self, map_raw=None, ops=None, *, region=None, fault=None):
        self.region=region or live.Region((0,0,5),(5,5,1))
        self.map_raw=map_raw if map_raw is not None else capture(changes={2+5*y:1 for y in range(5)})
        self.ops=ops if ops is not None else roster()
        self.fault=fault
        self.trace=[]; self.errors=[]; self.requests=0
        self.nonce=None; self.maps=0; self.pages=0; self.binds=0; self.released=False
        self.token=b'R'*16
        self.done=threading.Event()
        self.listener=socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
        self.listener.bind(('127.0.0.1',0)); self.listener.listen(1); self.listener.settimeout(5)
        self.address=self.listener.getsockname(); self.connection=None
        self.thread=threading.Thread(target=self.serve)

    def environment(self):
        env={key:value for key,value in os.environ.items() if not key.startswith('DFMCP_')}
        env.update({live.OPT_IN:'1',live.ENDPOINT:f'{self.address[0]}:{self.address[1]}',
                    live.MAP_TOKEN:'m'*32,wire.OPERATIONS_TOKEN:'o'*32})
        return env

    def __enter__(self):
        self.thread.start(); return self

    def __exit__(self,*_):
        self.thread.join(6)
        if self.thread.is_alive():
            if self.connection is not None:
                with contextlib.suppress(OSError): self.connection.shutdown(socket.SHUT_RDWR)
            self.listener.close(); self.thread.join(2)
        assert not self.thread.is_alive(), 'peer did not close'
        self.listener.close()
        if self.errors: raise self.errors[0]

    def read(self,n):
        out=bytearray()
        while len(out)<n:
            part=self.connection.recv(n-len(out))
            if not part: raise EOFError
            out+=part
        return bytes(out)

    def send(self,fields):
        raw=protobuf(fields)
        self.connection.sendall(struct.pack('<h2xi',-1,len(raw))+raw)

    def base(self,family,fields):
        assert fields[1]==(b'm'*32 if family=='map' else b'o'*32)
        assert fields[3]==1 and fields[4]==(5 if family=='map' else 4)
        if self.nonce is None: self.nonce=fields[2]
        assert fields[2]==self.nonce and len(self.nonce)==32
        if family=='map':
            assert set(fields)==set(range(1,12))
            assert tuple(fields[k] for k in range(5,11))==self.region.origin+self.region.size
            assert fields[11]==self.region.capture_limit
        else:
            assert {k:fields[k] for k in (5,6,7,8,11)}=={5:4096,6:4096,7:65536,8:16777216,11:65536}
        return {1:1,2:0,3:self.nonce,4:1,5:fields[4],6:23 if family=='map' else 7,
                7:b'df-test',8:b'dfhack-test'}

    def serve(self):
        try:
            self.connection,_=self.listener.accept()
            with self.connection:
                self.connection.settimeout(5)
                assert self.read(12)==b'DFHack?\n'+struct.pack('<i',1)
                self.connection.sendall((b'BadHack!\n' if self.fault=='greeting' else b'DFHack!\n')+struct.pack('<i',1))
                while True:
                    header=self.read(8)
                    method,size=struct.unpack('<h2xi',header)
                    assert 0<=size<=2048
                    fields=request_fields(self.read(size)); self.requests+=1
                    if method==0:
                        family,name=(('operations','Handshake'),('operations','ReadObservation'),
                                     ('map','Handshake'),('map','ReadObservation'))[self.binds]
                        plugin,package=({'operations':('dfmcp_operations_v1_4','dfmcp.operations.v1_4'),
                                         'map':('dfmcp_map_v1_5','dfmcp.map.v1_5')})[family]
                        assert fields=={1:name.encode(),2:(package+'.Request').encode(),
                                        3:(package+'.Reply').encode(),4:plugin.encode()}
                        self.trace.append(('bind',family,name)); self.binds+=1
                        self.send({1:2 if self.fault=='binding_alias' else self.binds+1})
                        continue
                    assert self.binds==4 and 2<=method<=5
                    family,name=(('operations','Handshake'),('operations','ReadObservation'),
                                 ('map','Handshake'),('map','ReadObservation'))[method-2]
                    reply=self.base(family,fields)
                    self.trace.append((family,name))
                    if name=='Handshake':
                        if family=='operations': assert set(fields)==set(range(1,9))|{11}
                        if family=='map' and self.fault=='software': reply[7]=b'other-df'
                        self.send(reply); continue
                    if family=='operations':
                        assert set(fields)==set(range(1,13))
                        if fields[12]==1:
                            assert fields[9]==self.token and fields[10]==0 and self.maps==1
                            assert self.pages==((len(self.ops)+65535)//65536)
                            self.trace.append(('release',))
                            if self.fault=='drop_release': break
                            reply[10]=b'X'*16 if self.fault=='release_token' else self.token
                            self.released=True
                        else:
                            assert fields[12]==0 and self.maps==1 and not self.released
                            offset=self.pages*65536
                            assert fields[10]==offset and fields[9]==(b'' if offset==0 else self.token)
                            data=self.ops[offset:offset+65536]
                            reply.update({9:data,10:self.token,11:offset,12:len(self.ops),
                                13:hashlib.sha256(self.ops).digest(),14:int(offset+len(data)==len(self.ops))})
                            if self.fault=='page_offset': reply[11]+=1
                            if self.fault=='page_digest': reply[13]=b'\0'*32
                            if self.fault=='page_token' and self.pages: reply[10]=b'X'*16
                            self.pages+=1
                    else:
                        self.maps+=1
                        assert self.maps<=2
                        if self.maps==2: assert self.released
                        reply[9]=self.map_raw
                        if self.fault=='map_changed' and self.maps==2:
                            reply[9]=self.map_raw[:-1]+bytes([self.map_raw[-1]^1])
                        if self.fault=='map_generation' and self.maps==2: reply[6]+=1
                        if self.fault=='map_nonce': reply[3]=b'N'*32
                        if self.fault=='unknown_field': reply[10]=b'bad'
                        if self.fault=='map_frame':
                            self.connection.sendall(struct.pack('<h2xi',-1,self.region.capture_limit+1025)); break
                        if self.fault=='notifications':
                            for _ in range(9): self.connection.sendall(struct.pack('<h2xi',-3,1)+b'x')
                        if self.fault=='map_nonminimal':
                            malformed=b'\x88\x00'+protobuf(reply)[1:]
                            self.connection.sendall(struct.pack('<h2xi',-1,len(malformed))+malformed)
                            continue
                    self.send(reply)
        except (EOFError,ConnectionResetError,BrokenPipeError):
            pass
        except BaseException as error:
            self.errors.append(error)
        finally:
            self.done.set()


class InventoryTests(unittest.TestCase):
    def call(self,peer,req=None,budget=None,**kwargs):
        with patch.dict(os.environ,peer.environment(),clear=True):
            return json.loads(live.run(req or request(Slot('bed','bed',(3,2,5))),peer.region,
                                      live.Authority.load(),budget or wire.Budget(10000),**kwargs))

    def test_native_bracket_picks_reachable_item_and_closes_before_solving(self):
        with Peer() as peer:
            solve=live.routes.allocate_routes
            def checked(*args,**kwargs):
                self.assertTrue(peer.done.wait(2),'native connection still held during solver')
                return solve(*args,**kwargs)
            with patch.object(live.routes,'allocate_routes',checked): out=self.call(peer)
        self.assertEqual(out['result']['assignments'][0]['item'],2)
        self.assertEqual(out['result']['total_route_steps'],3)
        self.assertTrue(out['native_capture_established'])
        self.assertTrue(out['capture_release_verified'])
        self.assertFalse(out['atomic_cross_profile_snapshot_proven'])
        self.assertEqual(out['source']['map']['generation'],23)
        self.assertEqual(out['source']['inventory']['generation'],7)
        self.assertEqual(peer.maps,2)
        self.assertEqual(peer.trace[-1],('map','ReadObservation'))
        self.assertEqual(FurniturePlan.from_json(out['result']['plan']).digest,out['result']['plan_digest'])
        # The core object remains unchanged; acquisition provenance is outside it.
        result=dict(out['result']); digest=result.pop('decision_digest')
        self.assertEqual(digest,hashlib.sha256(b'dfmcp-route-furniture-decision/1\0'+canonical(result)).hexdigest())
        self.assertNotIn('o'*32,json.dumps(out)); self.assertNotIn('m'*32,json.dumps(out))

    def test_paging_and_final_attachment_are_checked_before_any_selection(self):
        items=[dict(id=i,position=(4,4,5)) for i in range(1,3001)]
        raw=roster(items,jobs=({'id':1},),attachments=((1,1,0,-1),))
        self.assertGreater(len(raw),2*65536)
        with Peer(ops=raw) as peer: out=self.call(peer)
        self.assertGreater(peer.pages,2)
        self.assertEqual(out['result']['assignments'][0]['item'],2)
        self.assertEqual(out['source']['projection']['counts']['related_item'],1)
        self.assertEqual(out['source']['projection']['items'],3000)
        self.assertEqual(out['source']['inventory']['sha256'],hashlib.sha256(raw).hexdigest())

    def test_large_map_reply_uses_selection_bound_not_operations_page_bound(self):
        region=live.Region((0,0,5),(128,32,1))
        raw=capture(region.size,origin=region.origin)
        self.assertGreater(len(raw),wire.MAX_REPLY)
        with Peer(map_raw=raw,region=region) as peer: out=self.call(peer)
        self.assertEqual(out['result']['status'],'allocated')
        self.assertEqual(out['source']['map']['bytes'],len(raw))
        with self.assertRaises(ValueError): wire.decode(protobuf({9:raw}),9)
        self.assertEqual(wire.decode(protobuf({9:raw}),9,maximum_bytes=region.capture_limit+1024)[9],raw)
        for bound in (0,True,16777217):
            with self.assertRaises(ValueError): wire.decode(b'',maximum_bytes=bound)

    def test_all_projected_flags_use_only_ground_singletons(self):
        req=request(Slot('bed','bed',(1,1,5)))
        for flags in range(512):
            observed=decode_operations(roster([dict(id=1,flags=flags)]),lambda:None)
            candidates,counts=live.candidates_from(observed,req,lambda:None)
            self.assertEqual(len(candidates),int(flags==64),flags)
            self.assertEqual(sum(counts.values()),1)

    def test_related_and_unsupported_inventory_cannot_hide_behind_free_flags(self):
        values=[dict(id=1,container=2),dict(id=2),dict(id=3,holder=5),dict(id=4),
                dict(id=5,stack=2),dict(id=6,material=-1),dict(id=7,position=(-1,0,0)),
                dict(id=8,kind='OTHER'),dict(id=9),dict(id=10)]
        observed=decode_operations(roster(values,jobs=({'id':1},),buildings=({'id':5},),
                                          attachments=((1,4,0,-1),)),lambda:None)
        candidates,counts=live.candidates_from(observed,request(Slot('b','bed',(1,1,5)),excluded=(9,)),lambda:None)
        self.assertEqual([c.id for c in candidates],[10])
        self.assertEqual(counts,{'unsupported_kind':1,'projected_flags':0,'related_item':4,
            'non_singleton':1,'unknown_material':1,'invalid_position':1,'excluded':1,'candidate':1})

    def test_foreign_or_nonpaused_sources_never_produce_proposals(self):
        for ops in (roster(tick=4),roster(paused=False),roster(folder='other'),roster(site=3)):
            with self.subTest(kind=hashlib.sha256(ops).hexdigest()):
                with Peer(ops=ops) as peer:
                    with self.assertRaises(ValueError): self.call(peer)
        with Peer(map_raw=capture(paused=False)) as peer:
            with self.assertRaises(ValueError): self.call(peer)

    def test_substituted_region_and_invalid_unselected_inventory_are_refused(self):
        with Peer(map_raw=capture((4,5,1))) as peer:
            with self.assertRaises(ValueError): self.call(peer)
        with Peer(ops=roster([dict(id=1),dict(id=2,kind='OTHER',container=99)])) as peer:
            with self.assertRaises(ValueError): self.call(peer)

    def test_protocol_faults_stop_without_reconnection_or_second_capture(self):
        for fault in ('greeting','binding_alias','software','drop_release','release_token','page_offset',
                      'page_digest','map_changed','map_generation','map_nonce','unknown_field',
                      'map_frame','notifications','map_nonminimal'):
            with self.subTest(fault=fault):
                with Peer(fault=fault) as peer:
                    with self.assertRaises((ValueError,OSError)): self.call(peer)
                self.assertLessEqual(peer.maps,2)
        raw=roster([dict(id=i,position=(4,4,5)) for i in range(1,2001)])
        with Peer(ops=raw,fault='page_token') as peer:
            with self.assertRaises(ValueError): self.call(peer)
        self.assertEqual(peer.maps,1)
        self.assertFalse(peer.released)

    def test_acquisition_is_single_use_and_effect_method_refuses_before_contact(self):
        with Peer() as peer:
            with patch.dict(os.environ,peer.environment(),clear=True):
                with live.Client(live.Authority.load(),peer.region,wire.Budget(10000)) as client:
                    client.capture_once()
                    with self.assertRaises(ValueError): client.capture_once()
        self.assertEqual(peer.maps,2)
        with Peer() as peer:
            with patch.dict(os.environ,peer.environment(),clear=True):
                with live.Client(live.Authority.load(),peer.region,wire.Budget(10000)) as client:
                    with self.assertRaises(ValueError): client._call('build','CommitPlacement',{},set())
                    self.assertTrue(client.closed)
        self.assertEqual(peer.maps,0)
        self.assertFalse(any(row[0]=='build' for row in peer.trace))

    def test_mutation_admission_or_changed_read_authority_is_rejected(self):
        with patch.dict(os.environ,{live.OPT_IN:'1',wire.OPERATIONS_TOKEN:'o'*32,live.MAP_TOKEN:'m'*32},clear=True):
            authority=live.Authority.load()
            for key in ('DFMCP_BUILD_TOKEN','DFMCP_ADMITTED_BRIDGE_PROTOCOL','DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION'):
                with patch.dict(os.environ,{key:'1'}):
                    with self.assertRaises(ValueError): authority.guard()
            with patch.dict(os.environ,{live.MAP_TOKEN:'x'*32}):
                with self.assertRaises(ValueError): authority.guard()
            self.assertNotIn('o'*32,repr(authority)); self.assertNotIn('m'*32,repr(authority))
        for endpoint in ('localhost:5000','8.8.8.8:5000','127.0.0.1:05000','127.0.0.1:0'):
            with patch.dict(os.environ,{live.OPT_IN:'1',wire.OPERATIONS_TOKEN:'o'*32,live.MAP_TOKEN:'m'*32,live.ENDPOINT:endpoint},clear=True):
                with self.assertRaises(ValueError): live.Authority.load()

    def test_precontact_budget_refusal_does_not_open_socket(self):
        env={live.OPT_IN:'1',wire.OPERATIONS_TOKEN:'o'*32,live.MAP_TOKEN:'m'*32}
        with patch.dict(os.environ,env,clear=True):
            authority=live.Authority.load(); region=live.Region((0,0,5),(5,5,1))
            for name,value in [('calls',9),('network_bytes',1),('work_steps',0)]:
                budget=wire.Budget(10000); setattr(budget,name,value)
                with patch.object(live.socket,'socket',side_effect=AssertionError('contacted')):
                    with self.assertRaises(ValueError): live.Client(authority,region,budget)
            with self.assertRaises(ValueError): live.Region((0,0,5),(128,128,2))

    def test_solver_shares_exhaustible_budget_and_final_revocation_discards_result(self):
        with Peer() as peer:
            budget=wire.Budget(10000); budget.work_steps=100
            with self.assertRaises(ValueError): self.call(peer,budget=budget)
        self.assertTrue(peer.done.is_set())
        with Peer() as peer:
            original=live.bounded_output
            def revoke(value):
                raw=original(value)
                os.environ[live.MAP_TOKEN]='revoked-token-that-is-still-long-enough'
                return raw
            with patch.object(live,'bounded_output',revoke):
                with self.assertRaises(ValueError): self.call(peer)
        self.assertEqual(peer.maps,2)

    def test_post_acquisition_deadline_and_mid_solver_revocation_return_no_result(self):
        with Peer() as peer:
            budget=wire.Budget(10000)
            original=live.Client.capture_once
            def expired(client):
                result=original(client)
                budget.deadline=0
                return result
            with patch.object(live.Client,'capture_once',expired):
                with self.assertRaises(ValueError): self.call(peer,budget=budget)
        with Peer() as peer:
            original=live.routes.distances
            def revoke(*args,**kwargs):
                result=original(*args,**kwargs)
                os.environ[live.MAP_TOKEN]='x'*32
                return result
            with patch.object(live.routes,'distances',revoke):
                with self.assertRaises(ValueError): self.call(peer)
        self.assertEqual(peer.maps,2)

    def test_no_inventory_produces_checked_shortage_not_successful_partial_plan(self):
        with Peer(ops=roster([])) as peer: out=self.call(peer)
        self.assertTrue(out['ok'])
        self.assertEqual(out['result']['status'],'shortage')
        self.assertEqual(out['result']['shortage']['missing'],1)
        self.assertIsNone(out['result']['plan'])

    def test_hidden_target_remains_unknown_even_after_successful_acquisition(self):
        with Peer(map_raw=capture(changes={13:1})) as peer: out=self.call(peer)
        self.assertTrue(out['ok']); self.assertEqual(out['result']['status'],'target_unestablished')
        self.assertIsNone(out['result']['maximum_assignable']); self.assertIsNone(out['result']['shortage'])

    def test_cli_native_read_emits_one_json_and_never_modifies_request_or_creates_journal(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'request.json'; raw=canonical(request(Slot('bed','bed',(3,2,5))).json())
            path.write_bytes(raw)
            with Peer() as peer:
                process=subprocess.run([sys.executable,str(Path(cli.__file__)), '--request-file',str(path),
                    '--origin','0','0','5','--size','5','5','1'],env=peer.environment(),
                    capture_output=True,timeout=10)
            self.assertEqual(process.returncode,0,process.stderr)
            out=json.loads(process.stdout)
            self.assertEqual(out['result']['assignments'][0]['item'],2)
            self.assertEqual(process.stdout.count(b'\n'),1)
            self.assertEqual(process.stderr,b'')
            self.assertEqual(path.read_bytes(),raw)
            self.assertEqual(list(Path(directory).iterdir()),[path])

    def test_cli_fault_has_sanitized_null_result_and_no_partial_plan(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'private-request-name.json'; path.write_bytes(canonical(request(Slot('bed','bed',(3,2,5))).json()))
            with Peer(fault='drop_release') as peer:
                process=subprocess.run([sys.executable,str(Path(cli.__file__)),'--request-file',str(path),
                    '--origin','0','0','5','--size','5','5','1'],env=peer.environment(),capture_output=True,timeout=10)
            self.assertEqual(process.returncode,2)
            out=json.loads(process.stdout)
            self.assertFalse(out['ok']); self.assertIsNone(out['result'])
            self.assertIsNone(out['source']); self.assertIsNone(out['request'])
            self.assertNotIn(str(path).encode(),process.stdout)
            self.assertNotIn(b'o'*32,process.stdout); self.assertNotIn(b'm'*32,process.stdout)
            self.assertEqual(process.stderr,b'')

    def test_cli_invalid_shape_and_files_fail_before_socket(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); good=root/'good'; good.write_bytes(canonical(request(Slot('a','bed',(1,1,5))).json()))
            bad=root/'bad'; bad.write_text('{"schema":0,"schema":1}')
            link=root/'link'; link.symlink_to(good)
            fifo=root/'fifo'; os.mkfifo(fifo)
            oversized=root/'huge'; oversized.write_bytes(b'x'*16385)
            for path in (bad,link,fifo,oversized,root):
                with self.subTest(path=path.name):
                    with self.assertRaises((ValueError,OSError)): cli.read_request(str(path),wire.Budget(1000))
            with patch.object(cli,'run',side_effect=AssertionError('contacted')):
                out=io.BytesIO()
                with patch.object(cli.sys,'stdout',type('Output',(),{'buffer':out})()):
                    self.assertEqual(cli.main(['--request-file',str(good),'--origin','0','0','5','--size','1','1','0']),2)
                self.assertIsNone(json.loads(out.getvalue())['result'])

    def test_publication_short_write_never_retries_or_emits_second_object(self):
        class Short:
            calls=0
            def write(self,raw): self.calls+=1; return len(raw)-1
            def flush(self): raise AssertionError('flush after partial write')
        output=Short()
        with patch.object(cli.sys,'stdout',type('Output',(),{'buffer':output})()):
            self.assertEqual(cli.publish(b'{"ok":true}\n'),2)
        self.assertEqual(output.calls,1)
        with self.assertRaises(ValueError): live.bounded_output({'x':'x'*live.MAX_OUTPUT})


if __name__=='__main__': unittest.main()
