"""Production-codec and real original-journal companions for fresh room checks.

Native map/operations responses are fixture bytes, not DFHack execution. The CLI
cases replace only network acquisition; original codecs, private owners, packet
construction and environmental authority checks remain the production modules.
"""
from contextlib import redirect_stdout
from dataclasses import replace
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import furniture_batch
import room_readiness as readiness
import room_readiness_current as current
import room_readiness_fixtures as fixture
import room_readiness_rpc as rpc
import room_readiness_store as store
import room_terrain
import track_room_readiness as cli
from test_room_readiness_store import ADDRESS, create_room_batch, goal_from


class CurrentNativeCodecTests(unittest.TestCase):
    def setUp(self):
        self.goal=fixture.goal()
        progress=readiness.Progress(self.goal.digest)
        for tick in (300,310):
            progress=readiness.advance(readiness.begin_read(progress),self.goal,fixture.sample(self.goal,tick))
        self.history=progress

    def test_current_false_and_unknown_after_historical_success_preserve_goal(self):
        wall=min(room_terrain.selection(self.goal.room.room_plan).walls)
        before=self.history.view()
        for tile,truth in ((fixture.tile(3),'false'),(b'\1','unknown')):
            result=current.inspect(self.goal,fixture.sample(self.goal,320,overrides={wall:tile}),baseline=self.history)
            self.assertEqual(result['condition_truth'],truth)
            self.assertEqual(result['historical_basis']['phase'],'satisfied')
            self.assertEqual(self.history.view(),before)
        self.assertEqual(current.inspect(self.goal,fixture.sample(self.goal,10010),baseline=self.history)['condition_truth'],'true')

    def test_source_regression_or_missing_original_identity_withholds_positive_truth(self):
        values=(fixture.sample(self.goal,320,missing_building=2000),fixture.sample(self.goal,309),
                fixture.sample(self.goal,320,map_manifest=replace(fixture.MAP,generation=30)))
        for sample in values:
            result=current.inspect(self.goal,sample,baseline=self.history)
            self.assertEqual(result['status'],'invalidated_reference')
            self.assertEqual(result['condition_truth'],'unknown')
            self.assertEqual(len(result['assessments']),len(self.goal.condition.goals))

    def test_strict_original_access_survives_current_check_without_stability_credit(self):
        goal=replace(self.goal,require_circulation=True)
        entry=tuple(goal.room.room_plan.json()['areas'][0]['entry_point'])
        for tile,truth in ((fixture.tile(3,units=1),'unknown'),(fixture.tile(3,building=1),'false')):
            out=current.inspect(goal,fixture.sample(goal,320,overrides={entry:tile}))
            self.assertEqual(out['domain_truth']['circulation'],truth)
            self.assertEqual(out['condition_truth'],truth)
            self.assertFalse(out['stability_evaluated'])

    def test_complete_32_target_native_capture_and_malformed_bracket(self):
        goal=fixture.goal(True)
        sample=fixture.sample(goal,300,extra_items=2000)
        result=current.inspect(goal,sample)
        self.assertEqual(result['selected_furnishings'],32)
        self.assertEqual(result['condition_truth'],'true')
        self.assertLessEqual(len(current._canonical(result)),current.MAX_OUTPUT)
        changed=replace(sample,after_capture=fixture.map_capture(goal,301))
        self.assertRaises(ValueError,current.inspect,goal,changed)


class CurrentJournalCliTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='dfmcp-room-current-')
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        self.root.chmod(0o700)
        self.batch_path=self.root/'batch'
        self.origin=create_room_batch(self.batch_path)
        self.goal=goal_from(self.origin)
        self.path=self.root/'readiness'
        budget=rpc.Budget(60000)
        with store.Journal(str(self.path),budget,writable=True,create=(self.goal,ADDRESS)) as journal:
            with furniture_batch.Batch(str(self.batch_path),budget) as batch:
                journal.bind_batch(batch)
                for tick in (300,310):
                    journal.start_read()
                    journal.accept(fixture.sample(self.goal.readiness_goal,tick),lambda state: None)
        self.environment={rpc.OPT_IN:'1',rpc.ENDPOINT:'127.0.0.1:5000',rpc.MAP_TOKEN:'m'*32,
                         rpc.wire.BUILD_TOKEN:'b'*32,rpc.wire.OPERATIONS_TOKEN:'o'*32}

    def files(self):
        return {str(p.relative_to(self.root)):(p.stat().st_ino,p.stat().st_mode,hashlib.sha256(p.read_bytes()).hexdigest())
                for p in self.root.rglob('*') if p.is_file()}

    def run_check(self,sample,*,operation='check',output_hook=None):
        text=io.StringIO()
        output=cli.output if output_hook is None else output_hook
        with patch.dict(os.environ,self.environment,clear=True), patch.object(cli,'acquire',return_value=sample) as acquired, \
             patch.object(cli,'output',output), redirect_stdout(text):
            code=cli.main([operation,'--journal',str(self.path),'--timeout-ms','60000'])
        return code,json.loads(text.getvalue()),acquired.call_count

    def test_real_journal_bytes_and_original_effect_files_are_unchanged_by_check(self):
        before=self.files()
        wall=min(room_terrain.selection(self.goal.readiness_goal.room.room_plan).walls)
        sample=fixture.sample(self.goal.readiness_goal,320,overrides={wall:fixture.tile(3)})
        code,value,calls=self.run_check(sample)
        self.assertEqual((code,calls),(0,1),value)
        self.assertEqual(value['result']['current_condition_at_sample'],'false')
        self.assertTrue(value['result']['history']['room_readiness_sampled_condition'])
        self.assertEqual(self.files(),before)
        code,value,calls=self.run_check(sample,operation='inspect')
        self.assertEqual((code,calls),(0,0))
        self.assertEqual(value['result']['progress']['phase'],'satisfied')

    def test_failed_current_capture_preserves_every_original_byte_and_retries_nothing(self):
        before=self.files()
        sample=fixture.sample(self.goal.readiness_goal,320)
        sample=replace(sample,after_capture=fixture.map_capture(self.goal.readiness_goal,321))
        code,value,calls=self.run_check(sample)
        self.assertEqual((code,calls),(2,1))
        self.assertIsNone(value['result']['current_readiness'])
        self.assertEqual(self.files(),before)

    def test_final_environment_revocation_withholds_fresh_facts_without_journal_write(self):
        before=self.files()
        original=cli.output
        def output(value):
            raw=original(value)
            if value['result'].get('fresh_acquisition_completed'):
                os.environ.pop(rpc.MAP_TOKEN,None)
            return raw
        code,value,calls=self.run_check(fixture.sample(self.goal.readiness_goal,320),output_hook=output)
        self.assertEqual((code,calls),(2,1))
        self.assertIsNone(value['result']['current_readiness'])
        self.assertEqual(self.files(),before)

    def test_expired_goal_is_not_renewed_and_check_does_not_change_deadline(self):
        sample=fixture.sample(self.goal.readiness_goal,10010)
        before=self.path.read_bytes()
        code,value,calls=self.run_check(sample)
        self.assertEqual((code,calls),(0,1))
        self.assertEqual(value['result']['current_condition_at_sample'],'true')
        self.assertEqual(value['result']['history']['goal']['deadline_tick'],10000)
        self.assertFalse(value['result']['stability_evaluated'])
        self.assertEqual(self.path.read_bytes(),before)


if __name__ == '__main__':
    unittest.main()
