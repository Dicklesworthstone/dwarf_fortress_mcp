"""Run the complete CLI source with injected file/transport/authority owners.

No native codec, actual private-file custody, socket or DFHack execution is
claimed. The current-query module itself is NOT replaced in these tests.
"""
from __future__ import annotations

from contextlib import ExitStack, redirect_stdout
from dataclasses import dataclass, replace
import importlib.util
import io
import json
from pathlib import Path
from types import ModuleType, SimpleNamespace as NS
import sys
import unittest
from unittest.mock import patch

import test_room_readiness_current_isolated as fixture
import room_readiness_current as current

ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class Progress(fixture.Progress):
    map_witness: str | None = 'b'*64
    last_capture: str | None = 'd'*64

    @property
    def terminal(self):
        return self.phase in ('satisfied','failed','expired','cancelled','invalidated')

    def view(self):
        return {'phase':self.phase, 'streak':self.streak, 'read_outcome_unknown':self.reading,
                'sampled_condition_satisfied':self.phase=='satisfied'}


@dataclass(frozen=True)
class State:
    goal: object
    address: tuple
    progress: Progress
    frames: int = 5
    tail: bytes = b'a'*32


class Driver:
    def __init__(self, stack, phase='satisfied', strict=False):
        self.events = []
        self.allowed = True
        self.custody = True
        self.revoke_on_close = False
        self.native_hook = lambda: None
        self.sample_options = {}
        self.inner = fixture.Goal(require_circulation=strict)
        self.origin = NS(batch_path='/fixture/batch', room_handoff=NS(room_plan=self.inner.room.room_plan),
                         verify_batch=self.verify_batch)
        self.goal = NS(digest='f'*64, readiness_goal=self.inner, require_circulation=strict, origin=self.origin)
        self.state = State(self.goal, ('127.0.0.1',5000), Progress(self.goal.digest, phase=phase))
        self.budgets = []
        self.owners = []
        driver = self

        class Budget:
            def __init__(self, timeout):
                self.work_count = 0
                self.timeout = timeout
                driver.budgets.append(self)
            def remaining(self):
                self.work_count += 1
                return 60
            def work(self):
                self.remaining()

        class Authority:
            address = driver.state.address
            @classmethod
            def load(cls):
                driver.events.append('authority_load')
                if not driver.allowed:
                    raise ValueError('denied')
                return cls()
            def guard(self):
                driver.events.append('authority_guard')
                if not driver.allowed:
                    raise ValueError('revoked')

        class Batch:
            def __init__(self, path, budget):
                driver.events.append(('batch_open',path))
                self.budget = budget
            def close(self):
                driver.events.append('batch_close')
                if driver.revoke_on_close:
                    driver.allowed = False

        class Journal:
            def __init__(self, path, budget, *, writable=False, create=None):
                driver.events.append(('journal_open', writable, create))
                self.budget, self.state, self.length = budget, driver.state, 12345
                self.batch = None
                driver.owners.append(self)
            def bind_batch(self, batch):
                assert batch.budget is self.budget
                self.batch = batch
                self.check()
            def check(self):
                driver.events.append('journal_check')
                if not driver.custody:
                    raise ValueError('custody lost')
            def start_read(self):
                raise AssertionError('check must never start a durable monitoring read')
            def accept(self, sample, render):
                raise AssertionError('check must never publish a monitoring sample')
            def cancel(self, render):
                raise AssertionError('check must never cancel monitoring')
            def close(self):
                driver.events.append('journal_close')

        def acquire(authority, goal, budget):
            driver.events.append('native_acquire')
            assert goal is driver.inner
            assert budget is driver.budgets[-1]
            driver.native_hook()
            return fixture.Sample(goal, **driver.sample_options)

        def packet(operation,state,source_verified=False,native_contacted=False,
                   storage_acknowledged=False,verified=True,error=False):
            return {'ok':not error,'result':{
                'progress':state.progress.view() if state and verified else None,
                'identity':{} if state else None,
                'requested_plan':{'all_original_steps':True},
                'storage_acknowledged_this_call':storage_acknowledged,
                'native_connection_attempted':native_contacted,
            },'agent_turn':{
                'briefing':{},'coverage':{},'budget':{},'active_work':[],
                'references':[],'uncertainty':[],
            }}

        def add_arguments(parser):
            parser.add_argument('--wait-samples',type=int)
            parser.add_argument('--poll-ms',type=int)
        def parse_limits(args):
            if args.operation != 'wait' and (args.wait_samples is not None or args.poll_ms is not None):
                raise ValueError('wait policy only')
            return None

        def module(**attributes):
            out=ModuleType('injected_owner')
            out.__dict__.update(attributes)
            return out
        stack.enter_context(fixture.producers())
        sys.modules['room_readiness'].Progress=Progress
        sys.modules['room_readiness'].POLICY='legacy'
        sys.modules['room_readiness'].CIRCULATION_POLICY='strict'
        modules={
            'build_placement_wire':module(Rejected=ValueError,canonical=current._canonical,require=current._require),
            'furniture_batch':module(Batch=Batch),
            'furniture_completion':module(Origin=object),
            'room_readiness_rpc':module(Authority=Authority,Budget=Budget,acquire=acquire),
            'room_readiness_store':module(Goal=object,Journal=Journal,State=State,MAX_FILE=128*1024*1024,MAX_FRAMES=1030),
            'track_furniture_batch':module(packet=packet,separate_journal=lambda *a: None),
            'construction_wait':module(add_arguments=add_arguments,parse_limits=parse_limits,Limits=object),
        }
        sys.modules['construction_plan'].Goal=fixture.Goal
        stack.enter_context(patch.dict(sys.modules,modules))
        spec=importlib.util.spec_from_file_location('_room_current_cli', ROOT/'scripts/track_room_readiness.py')
        self.cli=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.cli)

    def verify_batch(self, batch):
        self.events.append('original_custody')
        if not self.custody:
            raise ValueError('original custody lost')

    def run(self, *extra, operation='check'):
        out=io.StringIO()
        with redirect_stdout(out):
            code=self.cli.main([operation,'--journal','/fixture/monitor',*extra])
        return code,json.loads(out.getvalue())


class RoomCheckCliTests(unittest.TestCase):
    def setUp(self):
        self.stack=ExitStack()
        self.addCleanup(self.stack.close)
        self.driver=Driver(self.stack)

    def test_terminal_check_reads_once_preserves_all_history_and_opens_read_only(self):
        d=self.driver
        before=repr(d.state)
        code,value=d.run()
        self.assertEqual(code,0,value)
        self.assertEqual(value['result']['current_condition_at_sample'],'true')
        self.assertEqual(value['result']['history']['progress']['phase'],'satisfied')
        self.assertNotIn('room_readiness_sampled_condition',value['result'])
        self.assertEqual(d.events.count('native_acquire'),1)
        self.assertIn(('journal_open',False,None),d.events)
        self.assertEqual(repr(d.state),before)
        self.assertIs(d.owners[0].state,d.state)
        self.assertGreater(d.budgets[0].work_count,10)
        self.assertFalse(value['result']['durable_sample_published'])
        self.assertFalse(value['result']['history']['storage_acknowledged_this_call'])

    def test_fresh_failure_cannot_be_hidden_by_old_success(self):
        d=self.driver
        d.sample_options={'terrain':{'wrong_shape':1}}
        code,value=d.run()
        self.assertEqual(code,0)
        self.assertTrue(value['result']['history']['room_readiness_sampled_condition'])
        self.assertEqual(value['result']['current_condition_at_sample'],'false')
        self.assertEqual(value['agent_turn']['briefing']['current_condition_at_sample'],'false')
        self.assertNotIn('room_readiness_sampled_condition',value['agent_turn']['briefing'])

    def test_check_does_not_resolve_a_prior_unfinished_monitor_read(self):
        d=self.driver
        d.state=replace(d.state,progress=replace(d.state.progress,phase='candidate',reading=True))
        code,value=d.run()
        self.assertEqual(code,0)
        self.assertTrue(d.state.progress.reading)
        self.assertTrue(value['result']['current_readiness']['historical_basis']['read_outcome_unknown'])
        self.assertEqual(value['result']['history']['progress']['effective_streak'],0)

    def test_expired_failed_cancelled_and_invalidated_histories_remain_terminal(self):
        d=self.driver
        for phase in ('expired','failed','cancelled','invalidated'):
            d.state=replace(d.state,progress=replace(d.state.progress,phase=phase))
            code,value=d.run()
            self.assertEqual(code,0)
            self.assertEqual(d.state.progress.phase,phase)
            expected='unknown' if phase=='invalidated' else 'true'
            self.assertEqual(value['result']['current_condition_at_sample'],expected)

    def test_original_policy_cannot_be_changed_and_wait_flags_are_refused_before_io(self):
        for flag in (('--require-circulation',),('--deadline-tick','999'),('--batch','/other'),
                     ('--interval-ticks','1'),('--wait-samples','2'),('--poll-ms','10')):
            d=self.driver
            d.events.clear()
            code,value=d.run(*flag)
            self.assertEqual(code,2)
            self.assertFalse(value['ok'])
            self.assertFalse(any(isinstance(e,tuple) and e[0]=='journal_open' for e in d.events))
            self.assertNotIn('native_acquire',d.events)

    def test_authority_endpoint_and_preflight_output_refusal_before_native_read(self):
        d=self.driver
        d.allowed=False
        self.assertEqual(d.run()[0],2)
        self.assertNotIn('native_acquire',d.events)
        d.allowed=True
        with patch.object(d.cli.Authority,'address',('127.0.0.1',5001)):
            self.assertEqual(d.run()[0],2)
        self.assertNotIn('native_acquire',d.events)
        with patch.object(d.cli,'MAX_OUTPUT',current.MAX_OUTPUT+4096):
            self.assertEqual(d.run()[0],2)
        self.assertNotIn('native_acquire',d.events)

    def test_failed_acquisition_never_retries_or_writes_monitor_state(self):
        d=self.driver
        def fail():
            raise OSError('lost response containing a secret')
        d.native_hook=fail
        before=repr(d.state)
        code,value=d.run()
        self.assertEqual(code,2)
        self.assertEqual(d.events.count('native_acquire'),1)
        self.assertEqual(repr(d.state),before)
        self.assertIsNone(value['result']['current_readiness'])
        self.assertNotIn('secret',json.dumps(value))

    def test_lost_custody_after_acquisition_withholds_fresh_facts(self):
        d=self.driver
        d.native_hook=lambda:setattr(d,'custody',False)
        code,value=d.run()
        self.assertEqual(code,2)
        self.assertIsNone(value['result']['current_readiness'])
        self.assertFalse(value['result']['fresh_acquisition_completed'])

    def test_revocation_during_inspection_and_after_close_withholds_response(self):
        d=self.driver
        d.native_hook=lambda:setattr(d,'allowed',False)
        code,value=d.run()
        self.assertEqual(code,2)
        self.assertIsNone(value['result']['current_readiness'])
        d.allowed=True
        d.native_hook=lambda:None
        d.revoke_on_close=True
        code,value=d.run()
        self.assertEqual(code,2)
        self.assertIsNone(value['result']['current_readiness'])
        self.assertEqual(d.events.count('native_acquire'),2)

    def test_final_serialization_revocation_and_output_bound_preserve_history(self):
        d=self.driver
        original=d.cli.output
        def output(value):
            raw=original(value)
            if value.get('result',{}).get('fresh_acquisition_completed'):
                d.allowed=False
            return raw
        with patch.object(d.cli,'output',output):
            code,value=d.run()
        self.assertEqual(code,2)
        self.assertIsNone(value['result']['current_readiness'])
        self.assertEqual(d.state.progress.phase,'satisfied')

    def test_unqualified_historical_inspect_and_terminal_sample_still_do_not_read(self):
        d=self.driver
        for operation in ('inspect','sample'):
            code,value=d.run(operation=operation)
            self.assertEqual(code,0)
            self.assertEqual(value['schema'],'dfmcp.room-readiness-monitor-result/1')
            self.assertNotIn('current_readiness',value['result'])
            self.assertTrue(value['result']['room_readiness_sampled_condition'])
        self.assertNotIn('native_acquire',d.events)

    def test_short_stdout_write_does_not_emit_second_object_or_repeat_acquisition(self):
        d=self.driver
        class Short(io.StringIO):
            calls=0
            def write(self,text):
                self.calls+=1
                return len(text)//2
        output=Short()
        with redirect_stdout(output):
            code=d.cli.main(['check','--journal','/fixture/monitor'])
        self.assertEqual(code,2)
        self.assertEqual(output.calls,1)
        self.assertEqual(d.events.count('native_acquire'),1)


if __name__ == '__main__':
    unittest.main()
