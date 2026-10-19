"""Original raw map evidence drives costs; no supplied tiles or source adapters."""
from copy import deepcopy
from pathlib import Path
import random
import struct
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'scripts'))
from furniture_allocation import Candidate, allocate
from furniture_plan import canonical
import excavation_observer as native
import room_layout as layout
import room_layout_observed as observed
from room_provisioning import RoomPlan
from test_room_layout_costs import hall, tiles, independent_best, stored_score

MANIFEST=native.Manifest(7,'synthetic-df','synthetic-dfhack')


def request(*areas, size=(8,3,1), **extra):
    return {'schema':layout.REQUEST_SCHEMA,'world_folder':'fixture','site':1,
            'region':{'origin':[10,10,2],'size':list(size)},'areas':list(areas) or [hall()],
            'objective':observed.EXCAVATION_OBJECTIVE,**extra}


def map_bytes(value, cells=None, *, changes=None, tick=100, paused=True, folder='fixture', site=1):
    size=value['region']['size']; low=value['region']['origin']; volume=size[0]*size[1]*size[2]
    cells=cells if cells is not None else ''.join('W' if x<4 else 'F' for _ in range(size[1]) for x in range(size[0]))
    if len(cells)!=volume:raise AssertionError('complete fixture map required')
    data=bytearray(b'DFMM1500'+struct.pack('>IIBIH',0,tick,int(paused),site,len(folder.encode()))+folder.encode())
    data+=struct.pack('>10I',128,128,8,*low,*size,volume)
    for index,cell in enumerate(cells):
        if cell in 'HM':
            data+=b'\1' if cell=='H' else b'\0'
        else:
            attrs=[1,2 if cell=='W' else 3,0,0,0,0,0,0,1,10000,10000]
            for field,number in (changes or {}).get(index,{}).items():attrs[field]=number
            data+=b'\2'+struct.pack('>IBBBBBBBIHH',*attrs)
    return bytes(data)


def solve(value, cells=None, **extra):
    return observed.solve(canonical(value),map_bytes(value,cells,**extra),MANIFEST)


class ObservedExcavationTests(unittest.TestCase):
    def test_reuses_existing_floor_instead_of_mining_an_equally_sized_hall(self):
        value=request();result=solve(value);body=result.json();evidence=body['excavation_evidence']
        self.assertEqual(evidence['observed_wall_targets'],0)
        self.assertEqual(evidence['observed_floor_targets'],12)
        self.assertEqual(evidence['first_feasible_wall_targets'],12)
        self.assertEqual(body['layout']['room_plan']['furniture_count'],2)
        self.assertEqual(body['layout']['room_plan']['intent']['areas'][0]['origin'],[14,10,2])
        self.assertEqual(body['capture_hex'],map_bytes(value).hex())
        self.assertEqual(evidence['map_witness'],body['source']['map_witness'])
        self.assertTrue(evidence['minimum_in_compiler_domain_proven'])
        self.assertFalse(evidence['actual_mining_work_observed'])
        self.assertFalse(evidence['native_mining_eligibility_proven'])
        self.assertFalse(body['native_acquisition_independently_attested'])
        self.assertEqual(observed.ObservedLayout.decode(result.encode()),result)
        nested=layout.Layout.decode(canonical(body['layout']))
        self.assertEqual(nested.room_plan().digest,body['layout']['room_plan']['plan_digest'])
        self.assertNotIn('excavation_costs',body['request'])
        self.assertEqual(body['request']['objective'],observed.EXCAVATION_OBJECTIVE)

    def test_caller_F_on_observed_wall_never_mints_zero_cost_floor(self):
        value=request();value['site_mask']={'region':deepcopy(value['region']),'cells':'F'*24}
        body=solve(value,'W'*24).json()
        self.assertEqual(body['effective_site_mask']['cells'],'F'*24)
        self.assertEqual(body['layout']['request']['excavation_costs'],'1'*24)
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'],12)
        self.assertEqual(body['excavation_evidence']['observed_floor_targets'],0)

    def test_caller_cannot_supply_costs_or_select_unbound_cost_objective(self):
        for objective in (observed.EXCAVATION_OBJECTIVE,layout.COST_OBJECTIVE,layout.MINIMUM_TARGETS_OBJECTIVE):
            value=request(objective=objective,excavation_costs='0'*24)
            with self.subTest(objective=objective),self.assertRaisesRegex(ValueError,'cannot import tile costs'):
                observed.prepare(canonical(value))
        with self.assertRaises(ValueError):observed.prepare(canonical(request(objective=layout.COST_OBJECTIVE)))
        with self.assertRaises(ValueError):layout.solve(canonical(request()))

    def test_64_raw_wall_floor_maps_match_independent_origin_rotation_enumeration(self):
        rng=random.Random(528)
        for case in range(64):
            cells=''.join(rng.choice('WF') for _ in range(20))
            value=request(hall(rotations=[0,90,180,270]),size=(5,4,1))
            result=solve(value,cells)
            explicit={**value,'objective':layout.COST_OBJECTIVE,'excavation_costs':''.join('1' if c=='W' else '0' for c in cells)}
            nested=layout.Layout.decode(canonical(result.json()['layout']))
            with self.subTest(case=case):self.assertEqual(stored_score(nested),independent_best(explicit))

    def test_unknown_missing_wet_designated_occupied_and_other_shapes_are_not_free(self):
        value=request(hall(origin=[14,10,2]))
        for cell,changes in [('H',None),('M',None),('F',{2:1}),('F',{3:1}),('F',{5:1}),
                             ('F',{6:1}),('F',{7:1}),('F',{1:6})]:
            cells=list('WWWWFFFF'*3);cells[4]=cell
            body=solve(value,''.join(cells),changes={4:changes} if changes else None).json()
            with self.subTest(cell=cell,changes=changes):
                self.assertEqual(body['status'],'no_layout_in_domain')
                self.assertIsNone(body['layout']['room_plan'])
                self.assertIsNone(body['excavation_evidence']['observed_wall_targets'])
                self.assertFalse(body['excavation_evidence']['minimum_in_compiler_domain_proven'])
                self.assertEqual(body['effective_site_mask']['cells'][4],'X')

    def test_all_floor_domain_cannot_invent_required_bedroom_walls(self):
        bed={'name':'bed','origin':[12,11,2],'template':{'kind':'bedroom_cluster','rooms_count':1,'room_size':[3,3]}}
        value=request(bed,size=(8,8,1))
        body=solve(value,'F'*64).json()
        self.assertEqual(body['status'],'no_layout_in_domain')
        self.assertIsNone(body['layout']['room_plan'])

    def test_bedroom_wall_ring_and_original_furnishings_remain_complete(self):
        bed={'name':'bed','origin':[12,11,2],'template':{'kind':'bedroom_cluster','rooms_count':1,'room_size':[3,3]}}
        value=request(bed,size=(8,8,1))
        result=solve(value,'W'*64)
        nested=result.json()['layout'];plan=RoomPlan.decode(canonical(nested['room_plan']))
        self.assertEqual(plan.json()['furniture_count'],3)
        body=plan.json();points=tiles(body)
        for unit in body['areas'][0]['units']:
            x,y,z=unit['origin'];w,h,_=unit['size'];door=tuple(unit['doorway'])
            ring={(xx,yy,z) for xx in range(x-1,x+w+1) for yy in range(y-1,y+h+1)
                  if xx in (x-1,x+w) or yy in (y-1,y+h)}-{door}
            self.assertFalse(points & ring)
        self.assertEqual(result.json()['excavation_evidence']['observed_wall_targets'],len(points))

    def test_two_rooms_and_access_are_charged_as_one_complete_union(self):
        value=request(hall('a',origin=[10,10,2]),hall('b',origin=[16,10,2]),size=(10,4,1),access_point=[10,13,2])
        cells=list('W'*40);cells[30]='F'
        result=solve(value,''.join(cells));body=result.json();points=tiles(body['layout']['room_plan'])
        self.assertEqual(body['excavation_evidence']['observed_floor_targets'],1)
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'],len(points)-1)
        self.assertTrue(body['access_evidence']['point_floor_observed'])
        self.assertIn((10,13,2),points)
        cells[30]='W'
        value['site_mask']={'region':value['region'],'cells':'F'*40}
        with self.assertRaisesRegex(ValueError,'access point must be observed'):solve(value,''.join(cells))

    def test_complete_32_item_eight_area_allocation_and_shortage(self):
        areas=[]
        for i in range(8):
            a=hall(str(i),origin=[10+(i%4)*7,10+(i//4)*3,2]);a['template'].update(table_count=2,columns=2);areas.append(a)
        value=request(*areas,size=(28,6,1))
        result=solve(value,'F'*168)
        plan=RoomPlan.decode(canonical(result.json()['layout']['room_plan']))
        candidates=tuple(Candidate(i,s.kind,s.target,(0,1),-1) for i,s in enumerate(plan.request().slots))
        self.assertEqual(len(allocate(plan.request(),candidates)['plan']['steps']),32)
        self.assertIsNone(allocate(plan.request(),candidates[:-1])['plan'])
        self.assertEqual(result.json()['excavation_evidence']['observed_wall_targets'],0)
        self.assertEqual(result.json()['excavation_evidence']['observed_floor_targets'],168)

    def test_whole_capture_identity_clock_and_pause_are_not_optional(self):
        value=request()
        for kwargs in ({'paused':False},{'folder':'other'},{'site':2}):
            with self.subTest(kwargs=kwargs),self.assertRaises(ValueError):solve(value,**kwargs)
        raw=map_bytes(value)
        for changed in (raw[:-1],raw+b'\0',raw.replace(b'DFMM1500',b'DFMM1501')):
            with self.assertRaises(ValueError):observed.solve(canonical(value),changed,MANIFEST)

    def test_node_limited_incumbent_remains_complete_but_not_optimal(self):
        value=request(search={'max_nodes':2})
        result=solve(value);body=result.json()
        self.assertEqual(body['status'],'layout_found')
        self.assertFalse(body['excavation_evidence']['minimum_in_compiler_domain_proven'])
        self.assertEqual(body['excavation_evidence']['observed_wall_targets'],12)
        self.assertEqual(observed.ObservedLayout.decode(result.encode()),result)

    def test_exact_shared_work_refuses_after_an_incumbent(self):
        value=request();used=solve(value).json()['work_units']
        for count in (1,used//2,used-1):
            with self.subTest(count=count),self.assertRaises(layout.WorkLimit):solve({**value,'search':{'max_work':count}})
        self.assertEqual(solve({**value,'search':{'max_work':used}}).json()['work_units'],used)

    def test_original_raw_evidence_and_cost_counts_cannot_be_resigned(self):
        result=solve(request())
        for field in ('costs','wall_count','raw','witness','objective'):
            body=result.json();body.pop('survey_digest')
            if field=='costs':body['layout']['request']['excavation_costs']='0'*24
            elif field=='wall_count':body['excavation_evidence']['observed_wall_targets']=1
            elif field=='raw':body['capture_hex']=map_bytes(request(),'W'*24).hex()
            elif field=='witness':body['excavation_evidence']['map_witness']='0'*64
            else:body['request']['objective']=layout.MINIMUM_TARGETS_OBJECTIVE
            candidate=observed.ObservedLayout(canonical(body))
            with self.subTest(field=field),self.assertRaises(ValueError):observed.ObservedLayout.decode(candidate.encode())

    def test_cancellation_every_checkpoint_preserves_input_and_returns_nothing(self):
        value=request(size=(4,3,1));before=deepcopy(value);raw=canonical(value);capture=map_bytes(value,'W'*12)
        calls=[];observed.solve(raw,capture,MANIFEST,lambda:calls.append(1))
        for stop in range(1,len(calls)+1):
            used=0
            def guard():
                nonlocal used
                used+=1
                if used==stop:raise ValueError('caller cancelled')
            with self.subTest(stop=stop),self.assertRaisesRegex(ValueError,'caller cancelled'):
                observed.solve(raw,capture,MANIFEST,guard)
        self.assertEqual(value,before)

    def test_costs_cover_the_maximum_original_capture_without_new_capacity(self):
        value=request(hall(origin=[10,10,2]),size=(32,32,1))
        result=solve(value,'F'*1024);nested=result.json()['layout']
        self.assertEqual(len(nested['request']['excavation_costs']),1024)
        self.assertLess(len(result.encode()),observed.MAX_OUTPUT)
        bad=request(size=(33,32,1))
        with self.assertRaises(ValueError):observed.prepare(canonical(bad))


if __name__=='__main__':unittest.main()
