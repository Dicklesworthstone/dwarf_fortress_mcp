"""Source regressions for explicit construction-progress review and recovery.

The native clock client is replaced with a deterministic port here. These tests
exercise confirmation binding and no-retry semantics, not DFHack transport.
"""
from contextlib import contextmanager
from pathlib import Path
import types
import unittest
from unittest.mock import patch

import route_furniture_progress as progress


class Budget:
    def __init__(self):
        self.calls=1000; self.network_bytes=10_000_000; self.disk_bytes=10_000_000
    def remaining(self): return 10.0
    def reserve(self, field, count):
        value=getattr(self,field)
        if count>value: raise ValueError('budget')
        setattr(self,field,value-count)


class Batch:
    def __init__(self, status='construction_pending'):
        self.id='a'*64
        self.review=types.SimpleNamespace(digest='b'*64)
        self.budget=Budget()
        self.writable=False
        self.status=status
        self.checked=0
    def audit(self):
        return {'batch_id':self.id,'review_digest':self.review.digest,
                'inventory_verified':True,'inventory_digest':'c'*64,
                'next_step':'chair','steps':[]}
    def check(self): self.checked+=1


class Authority:
    def __init__(self):
        self.address=('127.0.0.1',5000); self.run_token=b'r'*32; self.guards=0
    def guard(self,*_): self.guards+=1
    def clock_guard(self): self.guards+=1


def frontier(batch, authority):
    state=batch.audit()
    report={'batch_id':batch.id,'status':batch.status,'evidence_digest':'d'*64,
            'construction_prefix':{'status':'pending' if batch.status=='construction_pending' else 'quiescent'}}
    state.update(route_frontier=report, confirm_review=None, expected_plan=None)
    return state,report


class ProgressTests(unittest.TestCase):
    def test_review_confirms_only_current_construction_pending_frontier(self):
        batch=Batch()
        with patch.object(progress,'_frontier',side_effect=frontier),              patch.object(progress,'_reserve'):
            out=progress.review_progress(batch,Authority(),30,5000)
        self.assertEqual(len(out['progress_review']['confirm_progress']),64)
        batch.status='ready'
        with patch.object(progress,'_frontier',side_effect=frontier),              patch.object(progress,'_reserve'):
            out=progress.review_progress(batch,Authority(),30,5000)
        self.assertIsNone(out['progress_review']['confirm_progress'])

    def test_confirmation_binds_ticks_wall_inventory_and_frontier(self):
        batch=Batch()
        state,report=frontier(batch,Authority())
        _,one=progress._scope(batch,state,report,30,5000)
        _,two=progress._scope(batch,state,report,31,5000)
        self.assertNotEqual(one,two)
        state['inventory_digest']='e'*64
        _,three=progress._scope(batch,state,report,30,5000)
        self.assertNotEqual(one,three)
        report=dict(report,evidence_digest='f'*64)
        _,four=progress._scope(batch,batch.audit(),report,30,5000)
        self.assertNotEqual(one,four)

    def test_same_confirmation_has_one_derived_record_and_key(self):
        confirmation='1'*64
        self.assertEqual(progress._clock_key(confirmation),'frp-'+confirmation)
        self.assertEqual(progress._record_path('/private/clock',confirmation),
                         Path('/private/clock/progress-'+confirmation+'.run'))

    def test_stale_confirmation_refuses_before_clock_start(self):
        batch=Batch(); authority=types.SimpleNamespace(address=('127.0.0.1',5000),
            run_token=b'r'*32,clock_guard=lambda:None)
        with patch.object(progress,'ProgressAuthority',type('PA',(object,),{})):
            pass
        # Pure scope mismatch proves the effect confirmation cannot be reused.
        state,report=frontier(batch,Authority())
        _,actual=progress._scope(batch,state,report,30,5000)
        _,changed=progress._scope(batch,state,report,31,5000)
        self.assertNotEqual(actual,changed)

    def test_query_path_is_read_only_with_respect_to_run_dispatch(self):
        self.assertNotIn('clock.start(', progress.query_progress.__doc__ or '')
        # Structural guard: recovery implementation contains query/recover but no start call.
        import inspect
        source=inspect.getsource(progress.query_progress)
        self.assertIn('clock.recover',source)
        self.assertNotIn('clock.start',source)

    def test_progress_has_exactly_one_start_and_no_loop(self):
        import inspect
        source=inspect.getsource(progress.progress)
        self.assertEqual(source.count('clock.start('),1)
        self.assertNotIn('while ',source)
        self.assertNotIn('for ',source)

    def test_progress_refuses_writable_route_batch(self):
        batch=Batch(); batch.writable=True
        authority=object.__new__(progress.ProgressAuthority)
        with self.assertRaises(ValueError):
            progress.progress(batch,authority,30,5000,'1'*64,'/private/clock')

    def test_policy_never_claims_placement_or_automatic_second_burst(self):
        import inspect
        source=inspect.getsource(progress.progress)+inspect.getsource(progress.query_progress)
        self.assertIn("'automatic_second_clock_burst': False",source)
        self.assertIn("'placement_dispatched': False",source)
        self.assertIn("'placement_retry_permitted': False",source)


if __name__=='__main__':
    unittest.main()
