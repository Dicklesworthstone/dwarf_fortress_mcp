"""Actual combined solver/CLI with a deliberately injected acquisition port.

No producer algorithms or codecs are stubbed. Native acquisition, credentials,
network budget and operations projection are NOT qualified by this test suite.
"""
from __future__ import annotations

from dataclasses import replace
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from furniture_plan import canonical
import room_route_handoff as model
import room_route_workflow as flow
import review_room_routes as cli
import route_furniture_review as reviews
from test_room_route_handoff import fixture, raw_map, room_plan, source


class Authority:
    address = ('127.0.0.1', 5000)
    def __init__(self):
        self.denied, self.checks = False, 0
    def guard(self):
        self.checks += 1
        if self.denied:
            raise ValueError('revoked test authority')


class Port:
    def __init__(self, f):
        self.room, self.review, self.items, self.types, self.raw = f
        self.calls = []
        self.failure = None
    def __call__(self, request, origin, size, authority, budget, original=None):
        self.calls.append((request, origin, size, original, budget))
        if self.failure:
            raise self.failure
        budget.work()
        return self.raw, source(self.raw), self.items, self.types, {'candidate': len(self.items)}


def create(f=None, *, port=None, authority=None, budget=None, maximum=None):
    f = fixture() if f is None else f
    port = Port(f) if port is None else port
    authority = Authority() if authority is None else authority
    budget = cli.OfflineBudget(60000) if budget is None else budget
    origin, size = source(f[4]).origin, source(f[4]).size
    options = {} if maximum is None else {'max_route_steps': maximum}
    raw = flow.create(f[0], origin, size, authority, budget, acquire=port, **options)
    return json.loads(raw), port, authority, budget


class WorkflowTests(unittest.TestCase):
    def test_create_preserves_complete_room_and_original_request(self):
        f = fixture()
        value, port, _, budget = create(f)
        self.assertEqual(len(port.calls), 1)
        self.assertIs(port.calls[0][4], budget)
        self.assertIsNone(port.calls[0][3])
        self.assertEqual(value['result']['status'], 'ready_in_captured_model')
        h = model.RoomRouteHandoff.decode(canonical(value['handoff']), value['result']['handoff_digest'])
        room, review = h.unpack()
        self.assertEqual(room.encode(), f[0].encode())
        self.assertEqual(review.unpack()[1].json(), f[0].request().json())
        self.assertEqual(value['result']['allocation']['selection_policy'], flow.fixed.POLICY)
        self.assertEqual(value['result']['plan'], review.unpack()[3].json())
        self.assertIsNone(value['agent_turn']['anchor'])
        self.assertFalse(value['result']['placement_gate_enforced'])

    def test_check_never_allocates_or_replaces_missing_original_item(self):
        f = fixture(); v, _, _, _ = create(f)
        h = model.RoomRouteHandoff.decode(canonical(v['handoff']), v['result']['handoff_digest'])
        port = Port(f)
        old = port.items[0]
        port.items = (replace(old, id=900), *port.items[1:])
        port.types = {i.id: ('bed','chair','table').index(i.kind) for i in port.items}
        original = h.raw
        with patch.object(flow.fixed, 'allocate_fixed_order_routes', side_effect=AssertionError('no reallocation')):
            raw = flow.check(h, Authority(), cli.OfflineBudget(60000), acquire=port)
        value = json.loads(raw)
        self.assertEqual(value['result']['status'], 'unestablished')
        self.assertIsNone(value['result']['plan']); self.assertIsNone(value['handoff'])
        self.assertIsNone(value['result']['allocation'])
        self.assertEqual(port.calls[0][3], h.unpack()[1].unpack()[2])
        self.assertEqual(h.raw, original)

    def test_created_blocked_room_retains_intent_but_not_ready_plan(self):
        f = fixture(); p = Port(f)
        p.raw = raw_map(f[0], changes={(4,4,2): {'shape':3}})
        value, _, _, _ = create(f, port=p)
        self.assertEqual(value['result']['allocation']['status'], 'allocated')
        self.assertEqual(value['result']['status'], 'blocked_in_captured_model')
        self.assertIsNone(value['result']['plan']); self.assertIsNotNone(value['handoff'])
        h = model.RoomRouteHandoff.decode(canonical(value['handoff']), value['result']['handoff_digest'])
        repaired = json.loads(flow.check(h, Authority(), cli.OfflineBudget(60000), acquire=Port(f)))
        self.assertEqual(repaired['result']['status'], 'ready_in_captured_model')
        self.assertEqual(repaired['result']['handoff_digest'], h.digest)

    def test_shortage_exports_neither_partial_handoff_nor_plan(self):
        f = fixture(); p = Port(f); p.items = p.items[:1]; p.types = {p.items[0].id:p.types[p.items[0].id]}
        value, _, _, _ = create(f, port=p)
        self.assertEqual(value['result']['status'], 'shortage')
        self.assertIsNone(value['handoff']); self.assertIsNone(value['result']['handoff_digest'])
        self.assertIsNone(value['result']['plan']); self.assertIsNone(value['result']['assessment'])
        self.assertEqual(value['result']['room_plan'], f[0].json())

    def test_omitted_walls_limits_and_export_allowance_refuse_before_acquisition(self):
        f = fixture()
        for case in range(4):
            p = Port(f); b = cli.OfflineBudget(60000)
            origin, size, maximum = (0,0,2), (32,32,1), 100
            if case == 0: origin, size = (5,5,2), (3,3,1)
            elif case == 1: maximum = -1
            elif case == 2: b.work_steps = flow.EXPORT_WORK
            else: size = (128,128,128)
            with self.assertRaises(ValueError):
                flow.create(f[0], origin, size, Authority(), b, max_route_steps=maximum, acquire=p)
            self.assertEqual(p.calls, [])

    def test_wrong_original_endpoint_refuses_check_before_acquisition(self):
        f = fixture(); h = model.RoomRouteHandoff.create(f[0], f[1])
        a = Authority(); a.address = ('127.0.0.1', 5001); p = Port(f)
        with self.assertRaises(ValueError):
            flow.check(h, a, cli.OfflineBudget(60000), acquire=p)
        self.assertEqual(p.calls, [])

    def test_source_substitution_lost_read_or_revocation_never_retries(self):
        f = fixture()
        for case in range(5):
            p = Port(f); a = Authority(); calls = []
            def acquire(*args):
                calls.append(1)
                if case == 0: raise OSError('lost reply')
                raw, src, items, types, counts = p(*args)
                if case == 1: src = replace(src, endpoint='127.0.0.1:5002')
                if case == 2: src = replace(src, map_sha256='0'*64)
                if case == 3: src = replace(src, size=(31,32,1))
                if case == 4: a.denied = True
                return raw, src, items, types, counts
            with self.assertRaises((OSError, ValueError)):
                flow.create(f[0], (0,0,2), (32,32,1), a, cli.OfflineBudget(60000), acquire=acquire)
            self.assertEqual(calls, [1])

    def test_fixed_assignment_solves_blocked_initial_choice_without_changing_order(self):
        f = fixture(); p = Port(f)
        bed = next(i for i in p.items if i.kind == 'bed')
        table = next(i for i in p.items if i.kind == 'table')
        p.items = tuple(replace(i, position=bed.position) if i == table else i for i in p.items)
        p.items += (replace(table, id=900, position=(6,7,2)),)
        p.types[900] = p.types[table.id]
        value, _, _, _ = create(f, port=p)
        self.assertEqual(value['result']['status'], 'ready_in_captured_model')
        h = model.RoomRouteHandoff.decode(canonical(value['handoff']), value['result']['handoff_digest'])
        self.assertEqual(h.unpack()[1].unpack()[1].json(), f[0].request().json())
        self.assertEqual(next(s.item for s in h.unpack()[1].unpack()[3].steps if s.kind == 'table'), 900)

    def test_work_not_renewed_and_final_serialization_revocation_refuses(self):
        f = fixture(); b = cli.OfflineBudget(60000); b.work_steps = 300_000
        value, _, _, b = create(f, budget=b)
        result = value['result']
        self.assertLess(result['structural_work_reserved'], 300_000 - flow.EXPORT_WORK)
        self.assertLessEqual(result['structural_work_units'], result['structural_work_reserved'])
        self.assertLess(b.work_steps, flow.EXPORT_WORK)
        a = Authority(); original = flow.encode
        def revoke(v):
            out = original(v); a.denied = True; return out
        with patch.object(flow, 'encode', side_effect=revoke), self.assertRaises(ValueError):
            create(f, authority=a)

    def test_all_32_furnishings_fit_complete_workflow_output(self):
        f = fixture(room_plan(dining=True, count=16))
        value, _, _, _ = create(f)
        self.assertEqual(len(value['result']['plan']['steps']), 32)
        self.assertEqual(len(value['result']['assessment']['delivery_suffix']['routes']), 32)
        self.assertLessEqual(len(flow.encode(value)), flow.MAX_OUTPUT)


class CliTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.f = fixture()
        self.roomfile = self.root/'room.json'; self.roomfile.write_bytes(self.f[0].encode())
        self.reviewfile = self.root/'review.json'; self.reviewfile.write_bytes(self.f[1].raw)
        self.h = model.RoomRouteHandoff.create(self.f[0], self.f[1])
        self.handofffile = self.root/'handoff.json'; self.handofffile.write_bytes(self.h.raw)
    def tearDown(self):
        self.temp.cleanup()
    def args(self, op):
        if op in ('create', 'bind'):
            out = [op, '--room-plan-file', str(self.roomfile), '--expected-room-plan', self.f[0].digest]
            return out + (['--origin','0','0','2','--size','32','32','1'] if op == 'create' else
                          ['--review-file',str(self.reviewfile),'--expected-review',self.f[1].digest])
        return [op,'--handoff-file',str(self.handofffile),'--expected-handoff',self.h.digest]
    def invoke(self, args, *, port=None, authority=None, stdout=None):
        p = Port(self.f) if port is None else port
        a = Authority() if authority is None else authority
        output = io.BytesIO() if stdout is None else stdout
        with patch.object(sys, 'stdout', type('Output', (), {'buffer':output})()):
            status = cli.main(args, backend=lambda _: (cli.OfflineBudget(60000), a), acquire=p)
        return status, output.getvalue(), p

    def test_create_check_and_inspect_preserve_input_files(self):
        before = {p:p.read_bytes() for p in self.root.iterdir()}
        for op in ('create','check','bind','inspect'):
            status, raw, p = self.invoke(self.args(op))
            self.assertEqual(status, 0, raw)
            value = json.loads(raw); self.assertTrue(value['ok'])
            self.assertEqual(len(p.calls), int(op in ('create','check')))
        self.assertEqual(before, {p:p.read_bytes() for p in self.root.iterdir()})

    def test_check_rejects_all_creation_overrides_before_backend_contact(self):
        for extras in (['--origin','0','0','2'], ['--max-route-steps','0'], ['--sequence-deliveries'],
                       ['--room-plan-file','missing'], ['--emit','handoff']):
            out = io.BytesIO()
            with patch.object(sys,'stdout',type('Output',(),{'buffer':out})()):
                status = cli.main(self.args('check')+extras, backend=lambda _: self.fail('backend called'))
            self.assertEqual(status,2)
            self.assertEqual(json.loads(out.getvalue())['result'],None)

    def test_wrong_expected_room_and_handoff_digests_refuse_before_read_port(self):
        for op, flag in (('create','--expected-room-plan'),('check','--expected-handoff')):
            args = self.args(op); args[args.index(flag)+1] = '0'*64
            status, raw, p = self.invoke(args)
            self.assertEqual(status,2); self.assertEqual(p.calls,[])
            self.assertNotIn(str(self.root).encode(),raw)

    def test_narrow_create_export_requires_whole_condition(self):
        for blocked in (False,True):
            p = Port(self.f)
            if blocked: p.raw = raw_map(self.f[0],changes={(4,4,2):{'shape':3}})
            status, raw, _ = self.invoke(self.args('create')+['--emit','handoff'],port=p)
            self.assertEqual(status,2 if blocked else 0)
            if not blocked:
                self.assertEqual(model.RoomRouteHandoff.decode(raw,model.object_digest(raw)).unpack()[0].digest,self.f[0].digest)
            else:
                self.assertFalse(json.loads(raw)['ok'])

    def test_symlink_fifo_directory_oversize_or_replaced_input_refuses(self):
        symlink = self.root/'symlink'; symlink.symlink_to(self.handofffile)
        fifo = self.root/'fifo'; os.mkfifo(fifo)
        huge = self.root/'huge'; huge.write_bytes(b'X'*(model.MAX_BYTES+1))
        for p in (symlink,fifo,self.root,huge):
            args = self.args('inspect'); args[args.index('--handoff-file')+1] = str(p)
            status, raw, port = self.invoke(args)
            self.assertEqual(status,2); self.assertEqual(port.calls,[])
        original = os.read
        def replace_input(fd, n):
            result = original(fd,n)
            changed = self.root/'replacement'; changed.write_bytes(self.h.raw)
            os.replace(changed,self.handofffile)
            return result
        with patch.object(os,'read',side_effect=replace_input):
            status, _, _ = self.invoke(self.args('inspect'))
        self.assertEqual(status,2)

    def test_short_output_never_emits_second_packet(self):
        class Short(io.BytesIO):
            calls=0
            def write(self,data):
                self.calls += 1
                return super().write(data[:9])
        out=Short()
        status, raw, p = self.invoke(self.args('create'),stdout=out)
        self.assertEqual(status,2); self.assertEqual(out.calls,1); self.assertEqual(len(raw),9)
        self.assertEqual(len(p.calls),1)

    def test_true_offline_subprocess_bind_and_inspect_need_no_native_import(self):
        script = Path(cli.__file__)
        env = {'PATH':os.environ['PATH'],'PYTHONDONTWRITEBYTECODE':'1','PYTHONPATH':str(script.parent)}
        # This real interpreter rejects any attempt to create a socket/process
        # after startup; imports, file readers and all artifact codecs are real.
        runner = "import sys,runpy; sys.addaudithook(lambda e,a: (_ for _ in ()).throw(RuntimeError('I/O forbidden')) if e in ('socket.__new__','subprocess.Popen') else None); sys.argv=sys.argv[1:]; runpy.run_path(sys.argv[0],run_name='__main__')"
        bound = subprocess.run([sys.executable,'-W','error','-c',runner,str(script),*self.args('bind'),'--emit','handoff'],env=env,capture_output=True,timeout=20)
        self.assertEqual(bound.returncode,0,bound.stderr)
        self.assertEqual(bound.stdout,self.h.raw)
        viewed = subprocess.run([sys.executable,'-W','error','-c',runner,str(script),*self.args('inspect')],env=env,capture_output=True,timeout=20)
        self.assertEqual(viewed.returncode,0,viewed.stderr)
        self.assertEqual(json.loads(viewed.stdout)['result']['handoff_digest'],self.h.digest)
        self.assertEqual(viewed.stderr,b'')

    def test_source_failure_sanitizes_error_without_cached_room_or_tokens(self):
        p=Port(self.f); p.failure=OSError('secret token and private path')
        status, raw, _=self.invoke(self.args('create'),port=p)
        value=json.loads(raw)
        self.assertEqual(status,2); self.assertIsNone(value['result']); self.assertIsNone(value['handoff'])
        self.assertNotIn(b'secret',raw); self.assertNotIn(str(self.root).encode(),raw)
        self.assertEqual(len(p.calls),1)


if __name__ == '__main__':
    unittest.main()
