"""Executable offline layout/export pipeline using real files and real modules."""
from __future__ import annotations

import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_allocation import Candidate, Request, allocate
from furniture_plan import canonical
from room_provisioning import RoomPlan
import layout_rooms as cli
import room_layout as layout
from test_room_layout import bedroom, dining, request, solve

ROOT = Path(__file__).resolve().parents[1]


class CliTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='dfmcp-layout-test-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.path = self.root / 'request.json'
        self.doc = request(bedroom(count=2, item_constraints={'bed':{'material':[419,-1], 'max_distance':100}}),
                           dining(count=3, columns=3))
        self.path.write_bytes(canonical(self.doc))

    def run_cli(self, *args, source=True, env=None):
        return subprocess.run([sys.executable, str(ROOT/'scripts/layout_rooms.py'),
            *(['--request-file', str(self.path)] if source else []), *args],
            capture_output=True, timeout=15,
            env={**os.environ, 'PYTHONDONTWRITEBYTECODE':'1', **(env or {})})

    def test_checked_in_example_and_default_artifact_roundtrip(self):
        result=self.run_cli('--request-file',str(ROOT/'schemas/examples/automatic_room_layout_v1.json'),source=False)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(result.stderr,b'')
        decoded=layout.Layout.decode(result.stdout)
        self.assertEqual(decoded.room_plan().json()['furniture_count'],12)
        self.assertEqual(decoded.room_plan().json()['target_tiles'],59)
        self.assertFalse(result.stdout.endswith(b'\n'))
        self.assertEqual(set(self.root.iterdir()),{self.path})

    def test_every_export_is_exact_existing_format_with_all_constraints(self):
        full=solve(self.doc)
        plan=full.room_plan()
        expected={'layout':full.encode(),'room-plan':plan.encode(),
            'room-request':canonical(plan.json()['intent']),
            'excavation':canonical(plan.json()['excavation_blueprint']),
            'furniture-request':canonical(plan.json()['furniture_request'])}
        for kind, raw in expected.items():
            with self.subTest(kind=kind):
                result=self.run_cli('--emit',kind)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual(result.stdout,raw)
                self.assertEqual(result.stderr,b'')
        lowered=self.run_cli('--emit','room-plan').stdout
        self.assertEqual(RoomPlan.decode(lowered),plan)
        self.assertEqual(RoomPlan.from_request(self.run_cli('--emit','room-request').stdout),plan)
        furniture=Request.decode(self.run_cli('--emit','furniture-request').stdout)
        self.assertEqual(furniture,plan.request())
        candidates=tuple(Candidate(i,s.kind,s.target,s.material or (0,2),s.subtype if s.subtype is not None else -1)
                         for i,s in enumerate(furniture.slots))
        allocation=allocate(furniture,candidates)
        self.assertEqual(allocation['status'],'allocated')
        self.assertEqual(len(allocation['plan']['steps']),12)
        shortage=allocate(furniture,candidates[:-1])
        self.assertEqual((shortage['status'],shortage['plan'],shortage['assignments']),('shortage',None,[]))

    def test_saved_layout_replays_before_export_and_original_bytes_stay_unchanged(self):
        raw=self.run_cli().stdout
        path=self.root/'layout.json';path.write_bytes(raw)
        for kind in ('layout','room-plan','excavation','furniture-request'):
            result=self.run_cli('--layout-file',str(path),'--emit',kind,source=False)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(result.stdout,self.run_cli('--emit',kind).stdout)
            self.assertEqual(path.read_bytes(),raw)
        modified=json.loads(raw);modified['room_plan']['intent']['areas'][0]['origin'][0]+=1
        path.write_bytes(canonical(modified))
        refused=self.run_cli('--layout-file',str(path),'--emit','room-plan',source=False)
        self.assertEqual(refused.returncode,2)
        self.assertIsNone(json.loads(refused.stdout)['result'])

    def test_report_distinguishes_offline_proposal_from_native_facts(self):
        result=self.run_cli('--emit','report')
        self.assertEqual(result.returncode,0,result.stderr)
        out=json.loads(result.stdout);turn=out['agent_turn']
        self.assertTrue(out['ok'])
        self.assertEqual(turn['phase'],'propose')
        self.assertEqual(turn['briefing']['planned_furnishings'],12)
        self.assertIsNone(turn['anchor'])
        self.assertEqual(turn['coverage']['native_observations'],0)
        self.assertFalse(turn['briefing']['mutation_admissible'])
        self.assertTrue(turn['recommendations'][0]['native_survey_required'])
        self.assertLessEqual(len(result.stdout),cli.MAX_COMMAND_OUTPUT)

    def test_no_layout_and_search_limit_can_be_inspected_but_not_exported_as_plans(self):
        for doc, status, code in (
            (request(dining(),size=(3,3,1)),'no_layout_in_domain','ROOM_LAYOUT_NOT_FOUND'),
            (request(search={'max_nodes':1}),'search_limit_reached','ROOM_LAYOUT_SEARCH_LIMIT')):
            self.path.write_bytes(canonical(doc))
            result=self.run_cli()
            self.assertEqual(result.returncode,0)
            self.assertEqual(json.loads(result.stdout)['status'],status)
            self.assertIsNone(json.loads(result.stdout)['room_plan'])
            for kind in ('room-plan','room-request','excavation','furniture-request'):
                refused=self.run_cli('--emit',kind)
                self.assertEqual(refused.returncode,2)
                value=json.loads(refused.stdout)
                self.assertEqual(value['error']['code'],code)
                self.assertIsNone(value['result'])

    def test_no_native_module_or_network_use_even_with_ambient_credentials(self):
        # Audit hook observes the ACTUAL subprocess; it does not replace solver,
        # file reader, compiler, template producer, allocator or command source.
        code='''import runpy,sys
sys.argv=[sys.argv[1],*sys.argv[2:]]
def audit(event,args):
 if event.startswith('socket.') or event in ('subprocess.Popen','os.system'):
  raise RuntimeError('unexpected native or child process contact')
sys.addaudithook(audit)
sys.path.insert(0,str(__import__('pathlib').Path(sys.argv[0]).parent))
runpy.run_path(sys.argv[0],run_name='__main__')
'''
        result=subprocess.run([sys.executable,'-c',code,str(ROOT/'scripts/layout_rooms.py'),
            '--request-file',str(self.path)],capture_output=True,timeout=15,
            env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1','DFMCP_BUILD_TOKEN':'fixture-not-a-real-credential'})
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertNotIn(b'fixture-not-a-real-credential',result.stdout+result.stderr)
        self.assertEqual(result.stdout,solve(self.doc).encode())

    def test_real_symlink_fifo_directory_missing_and_oversize_inputs_refuse(self):
        link=self.root/'symlink';link.symlink_to(self.path)
        fifo=self.root/'fifo';os.mkfifo(fifo)
        large=self.root/'large';large.write_bytes(b' '* (layout.MAX_INPUT+1))
        for path in (link,fifo,self.root,self.root/'missing',large):
            result=self.run_cli('--request-file',str(path),source=False)
            self.assertEqual(result.returncode,2)
            self.assertIsNone(json.loads(result.stdout)['result'])
            self.assertNotIn(str(path).encode(),result.stdout+result.stderr)
        self.assertFalse((self.root/'missing').exists())

    def test_original_file_substitution_is_detected_during_read(self):
        replacement=self.root/'replacement'
        replacement.write_bytes(self.path.read_bytes())
        original_read=os.read
        switched=False
        def substitute(fd,n):
            nonlocal switched
            data=original_read(fd,n)
            if data and not switched:
                switched=True
                os.replace(replacement,self.path)
            return data
        with patch.object(cli.os,'read',substitute),self.assertRaisesRegex(ValueError,'changed'):
            cli.read_input(str(self.path),layout.MAX_INPUT,cli.Deadline(10000))

    def test_cli_work_limit_does_not_publish_a_prefix(self):
        self.path.write_bytes(canonical(request(search={'max_work':1})))
        out=self.run_cli('--emit','room-plan')
        self.assertEqual(out.returncode,2)
        value=json.loads(out.stdout)
        self.assertEqual(value['error']['code'],'ROOM_LAYOUT_WORK_LIMIT')
        self.assertIsNone(value['result'])

    def test_timeout_during_final_encoding_withholds_all_success_output(self):
        clock=[0.0];original_export=cli.export
        def late(*args):
            raw=original_export(*args);clock[0]=61.0;return raw
        stream=io.BytesIO()
        with patch.object(cli.time,'monotonic',lambda:clock[0]),patch.object(cli,'export',late),\
             patch.object(cli.sys,'stdout',type('Output',(),{'buffer':stream})()):
            code=cli.main(['--request-file',str(self.path),'--emit','room-plan'])
        self.assertEqual(code,2)
        value=json.loads(stream.getvalue())
        self.assertEqual(value['error']['code'],'ROOM_LAYOUT_TIMEOUT')
        self.assertIsNone(value['result'])

    def test_partial_stdout_write_does_not_emit_another_object_or_retry(self):
        class ShortOutput:
            writes=0
            def write(self,raw):
                self.writes+=1
                return len(raw)-1
            def flush(self):
                raise AssertionError('short write should not flush a second result')
        stream=ShortOutput()
        with patch.object(cli.sys,'stdout',type('Output',(),{'buffer':stream})()):
            code=cli.main(['--request-file',str(self.path)])
        self.assertEqual((code,stream.writes),(2,1))

    def test_closed_arguments_and_malformed_input_have_bounded_redacted_errors(self):
        for args in ([],['--request-file',str(self.path),'--layout-file',str(self.path)],
                     ['--request-file',str(self.path),'--execute'],
                     ['--request-file',str(self.path),'--timeout-ms','0'],
                     ['--request-file',str(self.path),'--emit','native-plan']):
            result=self.run_cli(*args,source=False)
            self.assertEqual(result.returncode,2)
            value=json.loads(result.stdout)
            self.assertIsNone(value['result'])
            self.assertEqual(result.stderr,b'')
        for raw in (b'{',b'\xff',b'',b'null',canonical(self.doc).replace(b'"site":2',b'"site":NaN')):
            self.path.write_bytes(raw)
            result=self.run_cli()
            self.assertEqual(result.returncode,2)
            self.assertIsNone(json.loads(result.stdout)['result'])

    def test_all_exports_keep_maximum_group_and_fit_output_limits(self):
        doc=request(*[dining(str(i),2,2) for i in range(8)],size=(32,32,1),
                    search={'max_work':layout.MAX_WORK})
        self.path.write_bytes(canonical(doc))
        for kind in cli.EXPORTS:
            result=self.run_cli('--emit',kind)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertLessEqual(len(result.stdout),cli.MAX_COMMAND_OUTPUT)
            value=json.loads(result.stdout)
            if kind=='room-plan':self.assertEqual(value['furniture_count'],32)
            if kind=='furniture-request':self.assertEqual(len(value['slots']),32)


class SchemaTests(unittest.TestCase):
    def test_schema_and_checked_in_example_match_runtime(self):
        from jsonschema import Draft202012Validator
        schema=json.loads((ROOT/'schemas/room_layout_request_v1.json').read_bytes())
        Draft202012Validator.check_schema(schema)
        validator=Draft202012Validator(schema)
        example=json.loads((ROOT/'schemas/examples/automatic_room_layout_v1.json').read_bytes())
        validator.validate(example)
        validator.validate(solve(example).json()['request'])
        for doc in (request(dining(origin=[10,10,2])),
                    request(bedroom(origin_region={'origin':[12,11,2],'size':[2,2,1]})),
                    request(dining(item_constraints={'chair':{'material':None,'subtype':None}}))):
            validator.validate(doc)
            validator.validate(solve(doc).json()['request'])

    def test_schema_and_runtime_reject_closed_shape_mutations(self):
        from jsonschema import Draft202012Validator
        schema=json.loads((ROOT/'schemas/room_layout_request_v1.json').read_bytes())
        validator=Draft202012Validator(schema)
        base=request(dining())
        cases=[]
        for key,value in [('schema','bad'),('areas',[]),('site',True),('execute',True),('focus',[1,2])]:
            d=copy.deepcopy(base);d[key]=value;cases.append(d)
        for key,value in [('name','bad/name'),('name','x\n'),('name','x'*25),('origin',[True,1,2]),
                          ('item_constraints',{'bed':{}}),('item_constraints',{'chair':{'quality':3}})]:
            d=copy.deepcopy(base);d['areas'][0][key]=value;cases.append(d)
        for key,value in [('table_count',0),('table_count',17),('columns',5),('room_size',[3,3]),('kind','bridge')]:
            d=copy.deepcopy(base);d['areas'][0]['template'][key]=value;cases.append(d)
        for value in ({'max_nodes':0},{'max_work':True},{'max_work':20000001},{'new_policy':1}):
            d=copy.deepcopy(base);d['search']=value;cases.append(d)
        d=copy.deepcopy(base);d['areas'][0].update(origin=[10,10,2],origin_region=d['region']);cases.append(d)
        for d in cases:
            self.assertTrue(list(validator.iter_errors(d)),d)
            with self.assertRaises((ValueError,TypeError)):solve(d)


if __name__=='__main__':
    unittest.main()
