"""Room intentions through exact geometry and allocation (bridge beads .3/.4/.5)."""
from __future__ import annotations

import copy
import itertools
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from furniture_allocation import Candidate, Request, allocate
from furniture_plan import FurniturePlan, canonical
from room_provisioning import INTENT_SCHEMA, MAX_PLAN_BYTES, RoomPlan


def bedroom(count=1, size=(3, 3), origin=(10, 10, 2), name='sleep'):
    return {'name': name, 'origin': list(origin), 'template': {
        'kind': 'bedroom_cluster', 'rooms_count': count, 'room_size': list(size)}}


def dining(count=1, columns=4, origin=(10, 20, 2), name='eat'):
    return {'name': name, 'origin': list(origin), 'template': {
        'kind': 'dining_hall', 'table_count': count, 'columns': columns}}


def intent(*areas):
    return {'schema': INTENT_SCHEMA, 'world_folder': 'region1', 'site': 2,
            'areas': list(areas) or [bedroom()]}


def mask(plan):
    selected = set()
    for part in plan.json()['excavation_blueprint']['parts']:
        x, y, z = part['region']['origin']
        w, h, levels = part['region']['size']
        points = {(xx, yy, zz) for xx in range(x, x+w) for yy in range(y, y+h) for zz in range(z, z+levels)}
        assert not selected & points
        selected |= points
    return selected


def reference_cluster(count, width, height, origin):
    # Independent direct cell-set formulation, not the compiler's rectangles.
    x, y, z = origin
    cells = set()
    for i in range(count):
        a, b = x + i % 4 * (width+1), y + i // 4 * (height+3)
        cells |= {(xx, yy, z) for xx in range(a, a+width) for yy in range(b, b+height)}
        cells.add((a+(width-1)//2, b+height, z))
    rows = (count+3)//4
    for r in range(rows):
        last_x = x + min(4, count-4*r)*(width+1)-2
        cells |= {(xx, y+r*(height+3)+height+1, z) for xx in range(x-1, last_x+1)}
    cells |= {(x-2, yy, z) for yy in range(y+height+1, y+(rows-1)*(height+3)+height+2)}
    return cells


def check_access(test, plan):
    floor = mask(plan)
    blocks = {s.target for s in plan.request().slots}
    test.assertTrue(blocks <= floor)
    for area in plan.json()['areas']:
        reached = {tuple(area['entry_point'])}
        frontier = set(reached)
        while frontier:
            following = set()
            for x, y, z in frontier:
                for p in ((x+1,y,z),(x-1,y,z),(x,y-1,z),(x,y+1,z)):
                    if p in floor and p not in blocks and p not in reached:
                        following.add(p)
            reached |= following
            frontier = following
        for a in area['approaches']:
            point = tuple(a['approach'])
            target = next(s.target for s in plan.request().slots if s.name == a['slot'])
            test.assertIn(point, reached)
            self_distance = sum(abs(a-b) for a,b in zip(point,target))
            test.assertEqual(self_distance, 1)


class RoomProvisioningTests(unittest.TestCase):
    def test_bedroom_exact_masks_and_access_all_counts_sizes(self):
        accepted = 0
        for n, w, h in itertools.product(range(1,11), range(3,8), range(3,8)):
            cells = reference_cluster(n, w, h, (10,10,2))
            lo = [min(p[i] for p in cells) for i in range(3)]
            hi = [max(p[i] for p in cells) for i in range(3)]
            volume = (hi[0]-lo[0]+1)*(hi[1]-lo[1]+1)
            if len(cells)>512 or volume>1024:
                with self.assertRaises(ValueError): RoomPlan.compile(intent(bedroom(n,(w,h))))
                continue
            p = RoomPlan.compile(intent(bedroom(n,(w,h))))
            self.assertEqual(mask(p), cells)
            self.assertEqual(len(p.request().slots), 3*n)
            self.assertEqual(len(p.json()['areas'][0]['units']), n)
            self.assertEqual(len(p.json()['excavation_blueprint']['parts']), 2*n+(n+3)//4+1)
            check_access(self, p)
            accepted += 1
        self.assertGreater(accepted,200)

    def test_dining_all_counts_columns_and_exact_32_slots(self):
        for n, columns in itertools.product(range(1,17), range(1,5)):
            p = RoomPlan.compile(intent(dining(n,columns)))
            expected = {(x,y,2) for x in range(10,10+min(n,columns)*3+1)
                        for y in range(20,20+((n+columns-1)//columns)*2+1)}
            self.assertEqual(mask(p), expected)
            self.assertEqual(len(p.request().slots), 2*n)
            self.assertEqual(len(p.json()['areas'][0]['units']), n)
            check_access(self,p)
            for unit in p.json()['areas'][0]['units']:
                slots = [s for s in p.request().slots if s.name in unit['slots']]
                self.assertEqual(sum(abs(a-b) for a,b in zip(slots[0].target,slots[1].target)),1)

    def test_multi_area_order_and_allocation_are_deterministic(self):
        a,b=bedroom(2),dining(3)
        a['item_constraints']={'chair':{'material':[10,-1]}}
        b['item_constraints']={'chair':{'material':[20,3],'max_distance':50}}
        request=intent(a,b); request['excluded_items']=[800,900]
        p=RoomPlan.compile(request)
        reverse=copy.deepcopy(request);reverse['areas'].reverse();reverse['excluded_items'].reverse()
        self.assertEqual(p.encode(),RoomPlan.compile(reverse).encode())
        candidates=tuple(Candidate(i+1,s.kind,s.target,s.material or (30,-1),s.subtype or -1)
                         for i,s in enumerate(p.request().slots))
        first=allocate(p.request(),candidates)
        self.assertEqual(first,allocate(p.request(),tuple(reversed(candidates))))
        self.assertEqual(first['status'],'allocated')
        compiled=FurniturePlan.from_json(first['plan'])
        self.assertEqual({s.name for s in compiled.steps},{s.name for s in p.request().slots})
        self.assertEqual(len(compiled.steps),12)

    def test_global_scarcity_does_not_feed_generic_area_first(self):
        a,b=dining(1,origin=(10,10,2),name='a'),dining(1,origin=(10,14,2),name='z')
        b['item_constraints']={'chair':{'material':[7,-1]}}
        p=RoomPlan.compile(intent(a,b))
        candidates=(Candidate(1,'chair',(11,11,2),(7,-1),-1),Candidate(2,'chair',(11,15,2),(9,-1),-1),
                    Candidate(3,'table',(12,11,2),(9,-1),-1),Candidate(4,'table',(12,15,2),(9,-1),-1))
        report=allocate(p.request(),candidates)
        self.assertEqual(report['status'],'allocated')
        choices={s['name']:s['item'] for s in report['plan']['steps']}
        self.assertEqual(choices['a.001.chair'],2)
        self.assertEqual(choices['z.001.chair'],1)
        shortage=allocate(p.request(),candidates[:1]+candidates[2:])
        self.assertEqual(shortage['status'],'shortage')
        self.assertIsNone(shortage['plan'])
        self.assertEqual(shortage['assignments'],[])

    def test_constraints_exclusions_and_dependency_edges_roundtrip(self):
        a=bedroom(2);a['item_constraints']={k:{'material':[12,2],'subtype':3,'max_distance':4}
                                          for k in ('bed','chair','table')}
        doc=intent(a);doc['excluded_items']=[42,11]
        p=RoomPlan.compile(doc)
        r=Request.decode(canonical(p.json()['furniture_request']))
        self.assertEqual(r,p.request())
        self.assertEqual(r.excluded_items,(11,42))
        for s in r.slots:
            self.assertEqual((s.material,s.subtype,s.max_distance),((12,2),3,4))
            previous={'bed':None,'chair':'bed','table':'chair'}[s.kind]
            self.assertEqual(s.after,() if previous is None else (s.name.rsplit('.',1)[0]+'.'+previous,))
        self.assertEqual(RoomPlan.decode(p.encode()),p)
        normalized=p.json()['intent']
        self.assertEqual(RoomPlan.from_request(canonical(normalized)),p)

    def test_excluded_regions_cover_room_doorway_corridor_and_other_levels(self):
        p=RoomPlan.compile(intent())
        for role_point in ((10,10,2),(11,13,2),(9,14,2),(8,14,2)):
            doc=intent();doc['excluded_regions']=[{'origin':list(role_point),'size':[1,1,1]}]
            with self.assertRaises(ValueError):RoomPlan.compile(doc)
        doc=intent();doc['excluded_regions']=[{'origin':[0,0,3],'size':[32768,32768,32765]}]
        self.assertEqual(mask(RoomPlan.compile(doc)),mask(p))
        doc['excluded_regions'][0]['origin'][2]=2
        doc['excluded_regions'][0]['size'][2]=32766
        with self.assertRaises(ValueError):RoomPlan.compile(doc)

    def test_excluded_order_normalized_duplicates_rejected(self):
        a={'origin':[30,30,2],'size':[2,2,1]};b={'origin':[40,40,2],'size':[2,2,1]}
        left=intent();left['excluded_regions']=[a,b]
        right=intent();right['excluded_regions']=[b,a]
        self.assertEqual(RoomPlan.compile(left),RoomPlan.compile(right))
        left['excluded_regions']=[a,a]
        with self.assertRaises(ValueError):RoomPlan.compile(left)

    def test_other_area_cannot_excavate_bedroom_wall(self):
        # The dining footprint does not overlap the bedroom floor, but opens its
        # north wall: a footprint-only collision check would incorrectly allow it.
        with self.assertRaisesRegex(ValueError,'bedroom wall'):
            RoomPlan.compile(intent(bedroom(),dining(1,origin=(10,7,2))))

    def test_overlap_counts_parts_capture_volume_and_full_halo_refused(self):
        cases=[intent(bedroom(),bedroom(name='other')),intent(bedroom(10),dining(2)),
               intent(bedroom(10,(7,7))),intent(bedroom(),dining(origin=(100,100,2))),
               intent(bedroom(origin=(2,10,2))),intent(bedroom(origin=(32766,10,2))),
               intent(bedroom(origin=(10,32766,2))),intent(bedroom(origin=(10,10,0))),
               intent(*[bedroom(name=str(i),origin=(10,10+i*6,2)) for i in range(9)])]
        for doc in cases:
            with self.subTest(doc=doc),self.assertRaises(ValueError):RoomPlan.compile(doc)
        valid=RoomPlan.compile(intent(bedroom(origin=(3,1,32766))))
        self.assertEqual(min(x for x,_,_ in mask(valid)),1)
        self.assertEqual(max(z for _,_,z in mask(valid)),32766)

    def test_closed_fields_exact_integer_types_and_malformed_templates(self):
        cases=[]
        for key,value in [('unexpected',True),('site',True),('world_folder','\0x'),('areas',()),('schema',1)]:
            doc=intent();doc[key]=value;cases.append(doc)
        for key,value in [('origin',[10,10,2.0]),('name','a'*25),('name','bad/key'),
                          ('item_constraints',{'other':{}}),('item_constraints',{'bed':{'quality':3}})]:
            doc=intent();doc['areas'][0][key]=value;cases.append(doc)
        for key,value in [('rooms_count',True),('rooms_count',0),('rooms_count',11),
                          ('room_size',[2,3]),('room_size',[3,8]),('room_size',[3,3,3]),
                          ('columns',4),('kind','workshop')]:
            doc=intent();doc['areas'][0]['template'][key]=value;cases.append(doc)
        for doc in cases:
            with self.subTest(doc=doc),self.assertRaises((ValueError,TypeError)):RoomPlan.compile(doc)

    def test_input_json_depth_duplicates_unicode_and_size(self):
        valid=canonical(intent())
        for raw in (b'', b' '*16385, valid.replace(b'"site":2',b'"site":2,"site":2'),
                    valid.replace(b'"site":2',b'"site":NaN'),b'['*11+b']'*11,
                    valid.replace(b'region1',b'\xff'),valid.replace(b'region1',b'\\ud800')):
            with self.subTest(raw=raw[:80]),self.assertRaises(ValueError):RoomPlan.from_request(raw)
        unicode_doc=intent();unicode_doc['world_folder']='region-\U0001f30d'
        self.assertEqual(RoomPlan.from_request(canonical(unicode_doc)).request().folder,unicode_doc['world_folder'])

    def test_plan_decode_rederives_every_semantic_output(self):
        p=RoomPlan.compile(intent())
        variants=[]
        for key,value in [('plan_digest','0'*64),('furniture_count',999),('terrain_observed',True),
                          ('target_tiles',100),('furniture_request_digest','0'*64),('policy','other')]:
            d=p.json();d[key]=value;variants.append(d)
        d=p.json();d['furniture_request']['slots'][0]['target'][0]+=1;variants.append(d)
        d=p.json();d['excavation_blueprint']['parts'].pop();variants.append(d)
        d=p.json();d['areas'][0]['approaches'][0]['approach'][0]+=1;variants.append(d)
        for d in variants:
            with self.subTest(d=d),self.assertRaises(ValueError):RoomPlan.decode(canonical(d))
        with self.assertRaises(ValueError):RoomPlan.decode(p.encode()+b'\n')
        with self.assertRaises(ValueError):RoomPlan.decode(b' '* (MAX_PLAN_BYTES+1))

    def test_digest_changes_with_original_constraints_and_exclusions(self):
        p=RoomPlan.compile(intent())
        docs=[]
        d=intent();d['excluded_items']=[42];docs.append(d)
        d=intent();d['areas'][0]['item_constraints']={'bed':{'max_distance':20}};docs.append(d)
        d=intent();d['areas'][0]['origin'][0]+=1;docs.append(d)
        d=intent();d['world_folder']='other';docs.append(d)
        d=intent();d['excluded_regions']=[{'origin':[20,20,2],'size':[1,1,1]}];docs.append(d)
        hashes={RoomPlan.compile(d).digest for d in docs}
        self.assertEqual(len(hashes),len(docs))
        self.assertNotIn(p.digest,hashes)

    def test_output_isolation_and_unknown_game_facts(self):
        p=RoomPlan.compile(intent())
        original=p.encode();out=p.json();out['intent']['areas'].clear()
        summary=p.summary();summary['areas'].clear()
        self.assertEqual(p.encode(),original)
        for field in ('terrain_observed','map_dimensions_observed','native_pathfinding_proven',
                      'existing_fort_access_proven','wall_preservation_observed','room_assignments_created',
                      'mutation_authority_granted','construction_completion_proven'):
            self.assertIs(p.json()[field],False)

    def test_guard_interruptions_before_any_partial_result(self):
        doc=intent(bedroom(2));calls=[]
        RoomPlan.compile(doc,lambda:calls.append(1))
        self.assertGreater(len(calls),200)
        for limit in range(len(calls)):
            count=0
            def guard():
                nonlocal count
                if count==limit:raise InterruptedError('cancelled')
                count+=1
            with self.assertRaises(InterruptedError):RoomPlan.compile(doc,guard)
        # Cancellation during bounded JSON scanning must reach the same owner.
        with self.assertRaises(InterruptedError):RoomPlan.from_request(canonical(doc),lambda:(_ for _ in ()).throw(InterruptedError()))

    def test_maximum_fields_within_complete_artifact_bounds(self):
        a=dining(16,name='A'*24);a['item_constraints']={
            k:{'material':[2147483647,2147483647],'subtype':2147483647,'max_distance':65532}
            for k in ('chair','table')}
        doc=intent(a);doc['world_folder']='\U0001f30d'*128;doc['site']=2147483647
        p=RoomPlan.compile(doc)
        self.assertLessEqual(len(p.encode()),MAX_PLAN_BYTES)
        self.assertLessEqual(len(canonical(p.json()['furniture_request'])),16384)
        self.assertEqual(len(p.request().slots),32)
        self.assertEqual(RoomPlan.decode(p.encode()),p)


if __name__ == '__main__':
    unittest.main()
