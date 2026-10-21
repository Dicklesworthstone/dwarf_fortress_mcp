"""Joint reducer/codec tests with explicit original-custody and native-fact ports.

Runs the unchanged construction_plan temporal engine and real map/review codecs.
Receipt decoding/assessment, the room compiler/terrain diagnosis and original
batch custody are injected producers, not native or filesystem qualification.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from contextlib import contextmanager
from dataclasses import dataclass, replace
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import sys
from types import ModuleType, SimpleNamespace as NS
import unittest
from unittest.mock import patch

from furniture_plan import FurniturePlan, Step, canonical, integer, require
from furniture_allocation import Candidate, Request
import route_furniture_allocation as routes
import route_furniture_review as reviews


class Cursor:
    def __init__(self, raw, maximum):
        require(type(raw) is bytes and 0 < len(raw) <= maximum, 'extent')
        self.raw, self.offset = raw, 0
    def take(self, n):
        require(0 <= n <= len(self.raw) - self.offset, 'truncated')
        start = self.offset
        self.offset += n
        return self.raw[start:self.offset]
    def number(self, n): return int.from_bytes(self.take(n), 'big')
    def u32(self, maximum=2**32-1): return integer(self.number(4), 0, maximum)
    def bounded(self, maximum): return self.take(integer(self.number(2), 1, maximum))
    def finish(self): require(self.offset == len(self.raw), 'trailing')


@dataclass(frozen=True)
class Manifest:
    generation: int
    df_version: str = 'df'
    dfhack_version: str = 'hack'
    @property
    def software(self): return self.df_version, self.dfhack_version
    def encode(self):
        integer(self.generation, 1, 2**64-2)
        return self.generation.to_bytes(8, 'big') + b''.join(field(s.encode()) for s in self.software)
    @classmethod
    def read(cls, reader):
        return cls(reader.number(8), reader.bounded(128).decode(), reader.bounded(128).decode())


def field(raw): return len(raw).to_bytes(2, 'big') + raw


@dataclass(frozen=True)
class Finding:
    status: str = 'condition_met'
    building_type: int = 1
    stage: int = 1


class Member:
    def __init__(self, raw, *timing):
        value = json.loads(raw)
        self.receipt = raw
        before = NS(generation=7, identity=(7, 1), dimensions=(100, 100, 8), tick=1)
        before.selection = NS(item=value['item'])
        self.record = NS(plan=NS(key=value['slot'], before=before, digest=b'p'*32),
            insertion=NS(building=value['item']+100, item=value['item'], job=value['item']+200,
                         pos=tuple(value['target'])))


def decode_operations(raw, guard):
    guard()
    v = json.loads(raw)
    return NS(**v, digest=hashlib.sha256(raw).hexdigest())


def assess(child, observed, guard):
    guard()
    return Finding(observed.statuses.get(str(child.record.insertion.item), 'condition_met'))


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def map_bytes(count=2, tick=2, wall=True, occupied=True, hidden=False, variation=0):
    width = count + 2
    raw = b'DFMM1500' + struct.pack('>IIBi', 0, tick, 1, 1) + field(b'fort')
    raw += struct.pack('>10I', 100, 100, 8, 0, 0, 5, width, 3, 1, width*3)
    for y in range(3):
        for x in range(width):
            if hidden and (x,y)==(0,0):
                raw += bytes([1]); continue
            shape = 2 if wall and (x,y)==(0,0) else 3
            building = int(occupied and y == 1 and 1 <= x <= count)
            raw += bytes([2]) + struct.pack('>IBBBBBBBIHH', 1+variation, shape, 0, 0, 0, 0, building, 0, 1, 10015, 10015)
    return raw


@contextmanager
def environment(count=2, **timing):
    receipt = ModuleType('construction_receipt')
    receipt.Cursor, receipt.Manifest, receipt.Goal = Cursor, Manifest, Member
    receipt.Assessment, receipt.Guard, receipt.Operations = Finding, object, object
    receipt.MAX_CAPTURE, receipt.MAX_OBSERVATIONS = 16*1024*1024, 512
    receipt.assess, receipt.decode_operations = assess, decode_operations
    wire = ModuleType('build_placement_wire')
    wire.MAX_RECORD_BYTES, wire.field, wire.integer, wire.require = 6144, field, integer, require
    names = ('construction_receipt', 'build_placement_wire', 'construction_plan',
             'room_route_handoff', 'route_furniture_completion', 'route_furniture_frontier',
             '_room_completion_under_test', '_construction_under_test')
    saved = {n:sys.modules.get(n) for n in names}
    try:
        sys.modules.update(construction_receipt=receipt, build_placement_wire=wire)
        construction = load('_construction_under_test', 'construction_plan.py')
        sys.modules['construction_plan'] = construction
        request = Request.from_json({'schema':'dfmcp.furniture-request/1', 'world_folder':'fort', 'site':1,
            'slots':[{'name':f's{i:02}', 'kind':'bed', 'target':[i+1,1,5]} for i in range(count)]})
        candidates = tuple(Candidate(i, 'bed', (i+1,2,5), (0,0), -1) for i in range(count))
        raw = map_bytes(count, tick=1, occupied=False)
        result = routes.allocate_routes(request, candidates, raw)
        source = reviews.Source('127.0.0.1:5000', 'df','hack',11,13,
            hashlib.sha256(raw).hexdigest(), 'a'*64,1,(100,100,8),(0,0,5),(count+2,3,1),(1000,1000,1000))
        review = reviews.create(request, result, source, {i:1 for i in range(count)})
        plan = FurniturePlan.from_json(result['plan'])
        records = tuple(canonical({'slot':s.name,'item':s.item,'target':list(s.target)}) for s in plan.steps)
        condition = construction.Goal(records, 100, **timing)

        class Original:
            def __init__(self, condition):
                self.condition = condition
                self.receipts = condition.receipts
                self.plan, self.review_digest, self.build = plan, review.digest, Manifest(7)
                self.operations_generation, self.batch_id, self.origin = 13, 'b'*64, '/original'
            def validate(self, guard):
                guard(); self.condition.encode()
                require(len(self.condition.receipts)==len(self.plan.steps),'partial originals')
            def encode(self, guard): self.validate(guard); return self.condition.encode()
            @classmethod
            def decode(cls, raw, guard): guard(); return cls(construction.Goal.decode(raw))

        class Parent:
            raw = b'complete original room'
            digest = hashlib.sha256(raw).hexdigest()
            def unpack(self, guard): guard(); return NS(digest='c'*64, json=lambda:{'original':'all'}), review
            def encode(self, guard): self.unpack(guard); return self.raw
            @classmethod
            def decode(cls, raw, expected, guard):
                guard(); require(raw==cls.raw and expected==cls.digest,'wrong parent'); return cls()

        rooms = ModuleType('room_route_handoff')
        rooms.MAX_BYTES, rooms.RoomRouteHandoff = 102400, Parent
        rooms.object_digest = lambda raw: hashlib.sha256(raw).hexdigest()
        def diagnosis(room, raw, guard):
            terrain = routes.decode_terrain(raw, guard)
            # An explicit terrain-producer port, not a second room compiler.
            reader = routes.Reader(raw); reader.take(21);reader.take(reader.number('H'));reader.take(40)
            presence = reader.number('B')
            shape = struct.unpack('>IBBBBBBBIHH',reader.take(19))[1] if presence==2 else None
            return {'all_required_shapes_at_sample':shape==2,'wall_shape':shape,
                    'map_sha256':terrain.digest,'observed_tick':terrain.tick}
        rooms._terrain_report = diagnosis
        child = ModuleType('route_furniture_completion')
        child.MAX_GOAL, child.Goal = construction.MAX_GOAL+20000, Original
        def verify(goal, batch):
            batch.check(); require(batch.original is goal and batch.complete,'original custody changed')
        child.verify_origin = verify
        child.from_batch = lambda batch, deadline, **kw: batch.original
        frontier = ModuleType('route_furniture_frontier')
        def binding(saved, before, guard):
            guard(); require(saved.digest==review.digest and before.selection.item in range(count),'selected item differs')
        frontier.bind_native_selection = binding
        sys.modules.update(room_route_handoff=rooms, route_furniture_completion=child,
                           route_furniture_frontier=frontier)
        model = load('_room_completion_under_test', 'room_route_completion.py')
        goal = model.Goal(Original(condition), Parent())
        def sample(tick=2, statuses=None, **kw):
            raw = map_bytes(count,tick,**kw)
            observed = canonical({'folder':'fort','site':1,'paused':True,'tick':tick,
                'horizons':[1000,1000,1000],'statuses':statuses or {}})
            child = construction.LinkedSample(Manifest(7), records, Manifest(13), observed, Manifest(7), records)
            return model.Sample(Manifest(11),raw,child,Manifest(11),raw)
        yield model, goal, sample
    finally:
        for n in names:
            if saved[n] is None: sys.modules.pop(n, None)
            else: sys.modules[n] = saved[n]


class CompletionTests(unittest.TestCase):
    def test_joint_success_requires_all_conditions_on_advancing_ticks(self):
        with environment() as (m,g,s):
            p=m.advance(m.begin_read(m.Progress(g.digest)),g,s())
            self.assertEqual((p.phase,p.streak),('candidate',1))
            p=m.advance(m.begin_read(p),g,s(3))
            self.assertEqual(p.phase,'satisfied')
            self.assertTrue(p.view()['sampled_whole_room_condition'])
            self.assertFalse(p.view()['room_completion_proven'])

    def test_room_defect_blocks_even_single_sample_completion(self):
        for option in ({'wall':False},{'hidden':True},{'occupied':False}):
            with self.subTest(option=option), environment(stable_samples=1,stable_span=0) as (m,g,s):
                p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(**option))
                self.assertEqual((p.phase,p.streak),('active',0))
                self.assertEqual(p.reason,'original_room_not_ready')

    def test_alternating_room_and_furniture_success_never_latches(self):
        with environment() as (m,g,s):
            p=m.Progress(g.digest)
            for tick in range(2,10):
                p=m.advance(m.begin_read(p),g,s(tick,wall=tick%2==0,
                    statuses={'0':'pending'} if tick%2==0 else None))
                self.assertFalse(p.terminal)
                self.assertEqual(p.streak,0)

    def test_wall_loss_resets_entire_streak(self):
        with environment() as (m,g,s):
            p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(2))
            p=m.advance(m.begin_read(p),g,s(3,wall=False))
            p=m.advance(m.begin_read(p),g,s(4))
            self.assertEqual((p.streak,p.first_tick), (1,4))
            self.assertEqual(m.advance(m.begin_read(p),g,s(5)).phase,'satisfied')

    def test_changed_map_at_same_tick_resets_even_unchanged_inventory(self):
        with environment() as (m,g,s):
            p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(2))
            p=m.advance(m.begin_read(p),g,s(2,variation=1))
            self.assertEqual((p.phase,p.streak),('active',0))
            self.assertEqual(p.reason,'room_sample_changed_at_same_tick')

    def test_repeated_tick_and_sample_gap_cannot_count_as_stability(self):
        with environment(max_gap=2) as (m,g,s):
            p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(2))
            p=m.advance(m.begin_read(p),g,s(2))
            self.assertEqual(p.streak,1)
            p=m.advance(m.begin_read(p),g,s(6))
            self.assertEqual((p.streak,p.first_tick),(1,6))

    def test_deadline_failure_and_invalidation_have_priority_over_room(self):
        for status,phase in (('removal_pending','failed'),('building_missing','invalidated'),('condition_met','expired')):
            with self.subTest(status=status), environment() as (m,g,s):
                p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(100,wall=False,statuses={'0':status}))
                self.assertEqual(p.phase,phase)

    def test_unknown_room_exhausts_original_sample_allowance(self):
        with environment(max_observations=2) as (m,g,s):
            p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(2,hidden=True))
            p=m.advance(m.begin_read(p),g,s(3,hidden=True))
            self.assertEqual((p.phase,p.reason),('expired','sample_budget_exhausted'))

    def test_interrupted_read_clears_streak_and_cancel_is_monitor_only(self):
        with environment() as (m,g,s):
            p=m.advance(m.begin_read(m.Progress(g.digest)),g,s())
            p=m.begin_read(m.begin_read(p))
            self.assertEqual((p.streak,p.interruptions),(0,1))
            p=m.cancel(p)
            self.assertEqual(p.phase,'cancelled')
            with self.assertRaises(ValueError): m.advance(p,g,s(3))

    def test_32_original_targets_are_jointly_required(self):
        with environment(32,stable_samples=1,stable_span=0) as (m,g,s):
            for i in range(32):
                p=m.advance(m.begin_read(m.Progress(g.digest)),g,s(statuses={str(i):'pending'}))
                self.assertNotEqual(p.phase,'satisfied')
                self.assertEqual(len(p.assessments),32)
            self.assertEqual(m.advance(m.begin_read(m.Progress(g.digest)),g,s()).phase,'satisfied')

    def test_source_tick_region_and_complete_receipt_binding(self):
        with environment() as (m,g,s):
            sample=s()
            mutations=[replace(sample,map_before=Manifest(12)),replace(sample,map_after=Manifest(12)),
                replace(sample,after_capture=map_bytes(tick=3)),
                replace(sample,furnishings=replace(sample.furnishings,operations=Manifest(14))),
                replace(sample,furnishings=replace(sample.furnishings,before_records=sample.furnishings.before_records[:1])),
                replace(sample,before_capture=map_bytes(tick=3),after_capture=map_bytes(tick=3))]
            for bad in mutations:
                with self.assertRaises(ValueError): m.advance(m.begin_read(m.Progress(g.digest)),g,bad)

    def test_goal_sample_roundtrips_and_all_truncated_prefixes_refuse(self):
        with environment() as (m,g,s):
            for obj,decode in ((g,m.Goal.decode),(s(),m.Sample.decode)):
                raw=obj.encode()
                self.assertEqual(decode(raw).encode(),raw)
                for cut in range(len(raw)):
                    with self.assertRaises(ValueError): decode(raw[:cut])
                with self.assertRaises(ValueError): decode(raw+b'x')

    def test_every_cancellation_callback_refuses_without_mutating_prior(self):
        with environment() as (m,g,s):
            p=m.begin_read(m.Progress(g.digest)); calls=0
            def count():
                nonlocal calls
                calls+=1
            m.advance(p,g,s(),count)
            before=p.view()
            for stop in range(1,calls+1):
                current=0
                def cancel():
                    nonlocal current
                    current+=1
                    if current==stop: raise ValueError('cancelled')
                with self.assertRaises(ValueError): m.advance(p,g,s(),cancel)
                self.assertEqual(p.view(),before)

    def test_original_parent_and_private_owner_are_rechecked(self):
        with environment() as (m,g,s):
            checked=[]
            batch=NS(original=g.original,room_handoff=g.handoff,complete=True,
                budget=NS(work=lambda:None),check=lambda:checked.append(True))
            m.verify_origin(g,batch)
            self.assertTrue(checked)
            batch.room_handoff=None
            with self.assertRaises(ValueError): m.verify_origin(g,batch)
            with self.assertRaises(ValueError): m.from_batch(batch,100)
            batch.room_handoff=g.handoff;batch.complete=False
            with self.assertRaises(ValueError): m.verify_origin(g,batch)


if __name__ == '__main__':
    unittest.main()
