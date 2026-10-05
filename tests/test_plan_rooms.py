"""Room recipe CLI and actual inventory/TCP handoff integration; no game effects."""
from __future__ import annotations

from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'scripts'))
import plan_rooms as cli
import furniture_inventory as inventory
from furniture_allocation import Request
from furniture_handoff import Handoff
from furniture_plan import canonical
from room_provisioning import RoomPlan
from room_inventory_peer import Peer, capture, TOKEN
from test_room_provisioning import intent, bedroom, dining


def clean_env(extra=None):
    return {**{k:v for k,v in os.environ.items() if not k.startswith('DFMCP_')},
            'PYTHONDONTWRITEBYTECODE':'1', **(extra or {})}


def invoke(*args, env=None):
    return subprocess.run([sys.executable,str(ROOT/'scripts/plan_rooms.py'),*args],
                          env=clean_env(env),capture_output=True,timeout=20)


class PlanRoomsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.request_file=Path(self.temp.name)/'rooms.json'
        self.document=intent(bedroom(2),dining(3))
        self.request_file.write_bytes(canonical(self.document))
        self.plan=RoomPlan.compile(self.document)

    def allocate(self, peer, budget=None):
        with patch.dict(os.environ,clean_env(peer.environment()),clear=True):
            return cli.allocate_plan(self.plan,inventory.Authority.load(),budget or inventory.Budget(10000))

    def test_offline_compile_exports_exact_canonical_consumer_artifacts(self):
        before=self.request_file.read_bytes()
        for emit,key in [('plan',None),('excavation','excavation_blueprint'),('furniture-request','furniture_request')]:
            result=invoke('compile','--request-file',str(self.request_file),'--emit',emit)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(result.stderr,b'')
            self.assertEqual(result.stdout,self.plan.encode() if key is None else canonical(self.plan.json()[key]))
        self.assertEqual(self.request_file.read_bytes(),before)
        self.assertEqual(RoomPlan.decode(invoke('compile','--request-file',str(self.request_file)).stdout),self.plan)
        self.assertEqual(Request.decode(invoke('compile','--request-file',str(self.request_file),
                                               '--emit','furniture-request').stdout),self.plan.request())

    def test_actual_cli_request_and_imported_plan_handoffs_match(self):
        plan_file=Path(self.temp.name)/'plan.json'
        compiled=invoke('compile','--request-file',str(self.request_file))
        plan_file.write_bytes(compiled.stdout)
        for key,path in [('--request-file',self.request_file),('--plan-file',plan_file)]:
            with Peer(capture(self.plan.request())) as peer:
                result=invoke('allocate',key,str(path),env=peer.environment())
                self.assertEqual(result.returncode,0,result.stdout+result.stderr)
                self.assertEqual(result.stderr,b'')
                value=json.loads(result.stdout)
                h=Handoff.from_json(value['result']['handoff'])
                self.assertEqual(h.request,self.plan.request())
                self.assertEqual(h.plan().json(),value['result']['plan'])
                self.assertEqual(value['result']['room_provisioning'],self.plan.summary())
                self.assertEqual(value['result']['source']['capture_sha256'],h.source.capture_sha256)
                self.assertEqual(peer.methods,['Handshake','ReadObservation'])
                self.assertEqual((peer.connections,len(peer.pages),peer.releases),(1,1,1))
                self.assertNotIn(TOKEN.encode(),result.stdout)
                self.assertIsNone(value['agent_turn']['anchor'])
                self.assertFalse(value['agent_turn']['briefing']['mutation_admissible'])

    def test_32_slots_with_paged_2000_item_inventory_shared_capture(self):
        self.plan=RoomPlan.compile(intent(bedroom(8),dining(4,origin=(10,25,2))))
        self.request_file.write_bytes(canonical(self.plan.json()['intent']))
        raw=capture(self.plan.request(),padding=2000)
        self.assertGreater(len(raw),65536)
        with Peer(raw) as peer:
            result=invoke('allocate','--request-file',str(self.request_file),'--timeout-ms','20000',env=peer.environment())
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
            value=json.loads(result.stdout)
            h=Handoff.from_json(value['result']['handoff'])
            self.assertEqual(len(h.plan().steps),32)
            self.assertEqual(h.request,self.plan.request())
            self.assertEqual(h.source.capture_sha256,hashlib.sha256(raw).hexdigest())
            self.assertEqual(peer.pages,list(range(0,len(raw),65536)))
            self.assertEqual(peer.releases,1)
            self.assertLessEqual(len(result.stdout),65536)
            print(f'ROOM_32 capture_bytes={len(raw)} output_bytes={len(result.stdout)}')

    def test_shortage_keeps_all_rooms_but_no_partial_executable_furniture(self):
        omitted=self.plan.request().slots[0].name
        with Peer(capture(self.plan.request(),omit=(omitted,))) as peer:
            result=json.loads(self.allocate(peer))
            self.assertTrue(result['ok'])
            value=result['result']
            self.assertEqual(value['status'],'shortage')
            self.assertIsNone(value['handoff']);self.assertIsNone(value['plan'])
            self.assertEqual(value['assignments'],[])
            self.assertEqual(value['room_provisioning'],self.plan.summary())
            self.assertEqual(len(value['request']['slots']),12)
            self.assertEqual(peer.releases,1)

    def test_wire_paging_source_and_release_faults_return_no_partial_result(self):
        for fault in ('lost_release','bad_release','bad_digest','page_offset','source_changed','extra_field','lost_page'):
            with self.subTest(fault=fault),Peer(capture(self.plan.request(),padding=1500),fault) as peer:
                result=invoke('allocate','--request-file',str(self.request_file),env=peer.environment())
                self.assertEqual(result.returncode,2,result.stdout)
                value=json.loads(result.stdout)
                self.assertFalse(value['ok']);self.assertIsNone(value['result'])
                self.assertEqual(peer.connections,1)
                self.assertNotIn(TOKEN.encode(),result.stdout+result.stderr)

    def test_foreign_fortress_and_corrupt_complete_roster_refused(self):
        for raw in (capture(self.plan.request(),folder='other'),capture(self.plan.request())+b'\0'):
            with Peer(raw) as peer:
                result=invoke('allocate','--request-file',str(self.request_file),env=peer.environment())
                self.assertEqual(result.returncode,2)
                self.assertIsNone(json.loads(result.stdout)['result'])
                self.assertEqual(peer.releases,1)

    def test_read_and_work_budgets_are_shared_and_not_renewed(self):
        b=inventory.Budget(10000)
        original=b.disk_bytes
        raw=cli.read_input(str(self.request_file),16384,b)
        self.assertEqual(original-b.disk_bytes,len(raw))
        b.work_steps=0
        with patch('socket.socket',side_effect=AssertionError('unexpected socket')):
            with self.assertRaises(ValueError):RoomPlan.from_request(raw,b.work)
        with Peer(capture(self.plan.request())) as peer:
            b=inventory.Budget(10000);b.calls=3
            with self.assertRaises(ValueError):self.allocate(peer,b)
            self.assertEqual(b.calls,0)
            self.assertEqual(peer.pages,[])
        with Peer(capture(self.plan.request())) as peer:
            b=inventory.Budget(10000)
            self.allocate(peer,b)
            self.assertEqual(b.calls,267) # Two bindings, handshake, one page, release.
            self.assertLess(b.work_steps,20_000_000)

    def test_changed_imported_plan_cannot_open_socket(self):
        value=self.plan.json();value.pop('plan_digest');value['furniture_count']=1
        forged=RoomPlan(canonical(value))
        with patch.dict(os.environ,clean_env({'DFMCP_ALLOW_UNADMITTED_FURNITURE_ALLOCATION':'1',
                      'DFMCP_OPERATIONS_PAGED_TOKEN':TOKEN}),clear=True):
            with patch('socket.socket',side_effect=AssertionError('unexpected socket')):
                with self.assertRaises(ValueError):cli.allocate_plan(forged,inventory.Authority.load(),inventory.Budget(10000))

    def test_handoff_enforces_future_material_distance_and_independent_generation(self):
        d=intent(bedroom());d['areas'][0]['item_constraints']={'bed':{'material':[7,-1],'max_distance':5}}
        self.plan=RoomPlan.compile(d)
        with Peer(capture(self.plan.request())) as peer:
            h=Handoff.from_json(json.loads(self.allocate(peer))['result']['handoff'])
        s=h.plan().steps[0]; row=next(r for r in h.selections if r.slot==s.name)
        h.validate_item(s,row.candidate,row.native_type,200,10000,1000)
        for candidate in (replace(row.candidate,material=(8,-1)),
                          replace(row.candidate,position=(100,100,2)),
                          replace(row.candidate,position=(10,10,3))):
            with self.assertRaises(ValueError):h.validate_item(s,candidate,row.native_type,200,10000,1000)
        h.validate_binding(h.source.address,h.request.folder,h.request.site,'DF-test','DFHack-test')
        self.assertEqual(h.source.generation,73)

    def test_final_serialization_revocation_withholds_cached_inventory(self):
        original=cli.encode_output
        def revoke(value):
            raw=original(value)
            os.environ[inventory.OPT_IN]='0'
            return raw
        with Peer(capture(self.plan.request())) as peer:
            with patch.object(cli,'encode_output',side_effect=revoke):
                with self.assertRaises(ValueError):self.allocate(peer)
            self.assertEqual(peer.releases,1)

    def test_revocation_during_cpu_projection_and_output_exhaustion(self):
        original=inventory.project
        def revoke(*args,**kwargs):
            os.environ[inventory.OPT_IN]='0'
            return original(*args,**kwargs)
        with Peer(capture(self.plan.request())) as peer:
            with patch.object(inventory,'project',side_effect=revoke):
                with self.assertRaises(ValueError):self.allocate(peer)
            self.assertEqual(peer.releases,1)
        with Peer(capture(self.plan.request())) as peer:
            with patch.object(cli,'MAX_OUTPUT',4096):
                with self.assertRaises(ValueError):self.allocate(peer)
            self.assertEqual(peer.releases,1)

    def test_input_special_files_symlinks_oversize_and_replacement(self):
        root=Path(self.temp.name)
        link=root/'link';link.symlink_to(self.request_file)
        fifo=root/'fifo';os.mkfifo(fifo)
        big=root/'big';big.write_bytes(b' '*16385)
        for path in (link,fifo,big,root):
            with self.subTest(path=path),self.assertRaises((OSError,ValueError)):
                cli.read_input(str(path),16384,inventory.Budget(10000))
        read=os.read
        def substitute(fd,count):
            raw=read(fd,count)
            replacement=root/'replacement';replacement.write_bytes(self.request_file.read_bytes())
            replacement.replace(self.request_file)
            return raw
        with patch.object(os,'read',side_effect=substitute):
            with self.assertRaises(ValueError):cli.read_input(str(self.request_file),16384,inventory.Budget(10000))

    def test_invalid_cli_inputs_and_environment_never_return_source_data(self):
        bad=Path(self.temp.name)/'bad-secret-path';bad.write_bytes(b'{"schema":null}')
        cases=[('allocate','--request-file',str(bad)),
               ('allocate','--request-file',str(self.request_file),'--emit','plan'),
               ('compile','--request-file',str(self.request_file),'--timeout-ms','0'),
               ('allocate','--request-file',str(self.request_file),'--plan-file',str(bad)),
               ('unrecognized-secret',)]
        for args in cases:
            result=invoke(*args)
            self.assertEqual(result.returncode,2)
            self.assertIsNone(json.loads(result.stdout)['result'])
            self.assertNotIn(b'bad-secret-path',result.stdout+result.stderr)
            self.assertNotIn(b'unrecognized-secret',result.stdout+result.stderr)
        for key in ('DFMCP_BUILD_TOKEN','DFMCP_BUILD_ALLOW_PLACE','DFMCP_ADMITTED_BRIDGE_PROTOCOL'):
            result=invoke('allocate','--request-file',str(self.request_file),
                          env={inventory.OPT_IN:'1','DFMCP_OPERATIONS_PAGED_TOKEN':TOKEN,key:'secret'})
            self.assertEqual(result.returncode,2)
            self.assertIsNone(json.loads(result.stdout)['result'])

    def test_swapped_handoff_or_request_identity_is_refused_after_read(self):
        original = inventory.project
        def wrong_request(*args, **kwargs):
            value = original(*args, **kwargs)
            value['request_digest'] = '0'*64
            return value
        def wrong_handoff(*args, **kwargs):
            value = original(*args, **kwargs)
            value['handoff']['request']['world_folder'] = 'other'
            return value
        for corrupt in (wrong_request, wrong_handoff):
            with Peer(capture(self.plan.request())) as peer:
                with patch.object(inventory, 'project', side_effect=corrupt):
                    with self.assertRaises(ValueError): self.allocate(peer)
                self.assertEqual(peer.releases, 1)

    def test_full_32_slot_maximum_width_source_result_fits_output(self):
        area = dining(16, name='A'*24)
        area['item_constraints'] = {kind: {'material': [2147483647,2147483647],
            'subtype':2147483647, 'max_distance':65532} for kind in ('chair','table')}
        doc = intent(area); doc['world_folder'] = '🌍'*128; doc['site'] = 2147483647
        self.plan = RoomPlan.compile(doc)
        with Peer(capture(self.plan.request())) as peer:
            raw = self.allocate(peer)
            value = json.loads(raw)
            self.assertEqual(Handoff.from_json(value['result']['handoff']).request, self.plan.request())
            self.assertLessEqual(len(raw), 65536)
            self.assertEqual(len(value['result']['plan']['steps']), 32)
            print(f'ROOM_MAX_FIELDS output_bytes={len(raw)}')

    def test_unchanged_native_allocation_default_still_works(self):
        with Peer(capture(self.plan.request())) as peer:
            with patch.dict(os.environ,clean_env(peer.environment()),clear=True):
                value=json.loads(inventory.run(self.plan.request(),inventory.Authority.load(),inventory.Budget(10000)))
            self.assertEqual(value['profile'],'furniture-allocation/1')
            self.assertNotIn('handoff',value['result'])
            self.assertNotIn('room_provisioning',value['result'])


if __name__=='__main__':unittest.main()
