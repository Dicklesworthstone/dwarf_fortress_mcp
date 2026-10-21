"""Execute the acquisition adapter with explicit transport/native method ports.

No real TCP, native framing or operations pagination is qualified. The production
adapter, source pinning, query ordering and close-before-validation do execute.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from contextlib import contextmanager
from dataclasses import replace
import sys
from types import ModuleType, SimpleNamespace as NS
import unittest
from unittest.mock import patch

from furniture_plan import require
from test_room_route_completion import environment, load, Manifest
from test_room_route_completion_runtime import Budget, endpoint


@contextmanager
def transport():
    with environment() as (model,goal,sample), patch.dict(sys.modules):
        events=[]
        flags=NS(map_generation=11,build_generation=7,inventory_generation=13,
                 changed_map=False,alias=False,revoke_on_close=False,fail_inventory=False)
        class Authority:
            address=('127.0.0.1',5000)
            revoked=False
            def guard(self): require(not self.revoked,'revoked')
        auth=Authority()
        source=sample()
        class Base:
            def __init__(self,authority,condition,budget):
                events.append('connect');self.authority=authority;self.budget=budget
                self.member_goals=condition.goals;self.used=False;self.closed=False
                self.manifests={'build':Manifest(flags.build_generation),'operations':Manifest(flags.inventory_generation)}
                self.methods={('build','QueryPlacement'):2,('operations','ReadObservation'):3}
            def _frame(self,method,payload):
                require(method==0,'only binding through fake frame port')
                events.append(('bind',payload[1]))
                return {1:2 if flags.alias else 20+len(self.methods)}
            def _member_receipt(self,member):
                events.append(('receipt',member.record.plan.key));return member.receipt
            def _capture_operations(self):
                events.append('operations_complete_release')
                if flags.fail_inventory:raise OSError('incomplete inventory release')
                return source.furnishings.capture
            def close(self):
                if not self.closed: events.append('close')
                self.closed=True
                if flags.revoke_on_close:auth.revoked=True
            def __enter__(self):return self
            def __exit__(self,*_):self.close()
        class MapPort:
            def _map_frame(self,*_):raise AssertionError('wire framing not exercised')
            def _map_call(self,observe):
                require(not self.closed,'closed')
                self.map_manifest=Manifest(flags.map_generation)
                if not observe:events.append('map_handshake');return None
                self.map_reads+=1
                require(self.map_reads<=2,'read cap')
                events.append('map_read')
                raw=source.before_capture
                if flags.changed_map and self.map_reads==2:raw=raw[:-1]+bytes([raw[-1]^1])
                return raw
        construction=ModuleType('construction_plan_rpc');construction.Client=Base
        wire=ModuleType('construction_monitor_rpc')
        wire.encode=lambda value:value;wire.decode=lambda value,maximum:value;wire.endpoint=endpoint
        room=ModuleType('room_readiness_rpc')
        room.Authority,room.Budget,room.MAX_RPC_CALLS=Authority,Budget,332
        room.Client,room.MAP_METHODS=MapPort,('Handshake','ReadObservation')
        sys.modules.update(construction_plan_rpc=construction,construction_monitor_rpc=wire,
                           room_readiness_rpc=room,room_route_completion=model)
        rpc=load('_room_rpc_test','room_route_completion_rpc.py')
        yield NS(rpc=rpc,model=model,goal=goal,auth=auth,events=events,flags=flags)


class AcquisitionTests(unittest.TestCase):
    def test_all_original_receipts_bracket_maps_and_one_inventory_then_close(self):
        with transport() as t:
            original=t.model.Sample.validate
            def validate(sample,goal,guard):
                self.assertEqual(t.events[-1],'close')
                return original(sample,goal,guard)
            with patch.object(t.model.Sample,'validate',validate):
                result=t.rpc.acquire(t.auth,t.goal,Budget())
            self.assertEqual(t.events,['connect',('bind',b'Handshake'),('bind',b'ReadObservation'),
                'map_handshake',('receipt','s00'),('receipt','s01'),'map_read',
                'operations_complete_release','map_read',('receipt','s00'),('receipt','s01'),'close'])
            self.assertEqual(result.before_capture,result.after_capture)

    def test_changed_original_sources_close_before_reading_game_data(self):
        for field in ('map_generation','build_generation','inventory_generation'):
            with self.subTest(field=field),transport() as t:
                setattr(t.flags,field,99)
                with self.assertRaises(ValueError):t.rpc.acquire(t.auth,t.goal,Budget())
                self.assertEqual(t.events[-1],'close')
                self.assertNotIn('operations_complete_release',t.events)
                self.assertFalse(any(isinstance(e,tuple) and e[0]=='receipt' for e in t.events))

    def test_endpoint_mismatch_refuses_before_connection(self):
        with transport() as t:
            t.auth.address=('127.0.0.1',5001)
            with self.assertRaises(ValueError):t.rpc.acquire(t.auth,t.goal,Budget())
            self.assertEqual(t.events,[])

    def test_binding_alias_and_inventory_failure_never_retry(self):
        for field in ('alias','fail_inventory'):
            with self.subTest(field=field),transport() as t:
                setattr(t.flags,field,True)
                with self.assertRaises((ValueError,OSError)):t.rpc.acquire(t.auth,t.goal,Budget())
                self.assertEqual(t.events.count('connect'),1)
                self.assertEqual(t.events[-1],'close')

    def test_changed_map_endpoint_rejects_after_source_close(self):
        with transport() as t:
            t.flags.changed_map=True
            with self.assertRaises(ValueError):t.rpc.acquire(t.auth,t.goal,Budget())
            self.assertEqual(t.events[-1],'close')
            self.assertEqual(t.events.count('map_read'),2)

    def test_authority_loss_after_close_cannot_publish_cached_evidence(self):
        with transport() as t:
            t.flags.revoke_on_close=True
            with self.assertRaises(ValueError):t.rpc.acquire(t.auth,t.goal,Budget())
            self.assertEqual(t.events[-1],'close')

    def test_connection_can_acquire_only_one_complete_sample(self):
        with transport() as t:
            with t.rpc.Client(t.auth,t.goal,Budget()) as client:
                client.capture_once();count=len(t.events)
                with self.assertRaises(ValueError):client.capture_once()
                self.assertEqual(len(t.events),count)


if __name__=='__main__':
    unittest.main()
