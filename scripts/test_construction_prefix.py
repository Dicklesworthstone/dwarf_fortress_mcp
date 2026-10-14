"""Actual construction predicate/codecs over independently encoded fixture bytes."""
from dataclasses import replace
import hashlib
import struct
import unittest

from build_placement_wire import Capture, Insertion, Item, Plan, Record, Selection, Tile, MAX_TICK
from construction_receipt import Goal, assess, assess_placed_receipt, decode_operations
from construction_prefix import assess_prefix


def idle():
    pass


def receipt(index=0, *, kind=1, tick=10):
    tile = Tile(2, 1, 3)
    before = Capture(11, index, tick, 2, (128, 128, 10), 200 + index, 100 + index, index,
        'region1', True, True, True, Selection(kind, 10 + index, 2 + index, 2, 5), (tile,) * 9,
        Item(presence=2, pos=(1, 1, 5), kind=kind, native_type=30 + kind,
             subtype=-1, material=419, material_index=-1, on_ground=True, ground=tile))
    plan = Plan('prefix-' + str(index), before)
    insertion = Insertion(before.next_building, before.next_job, before.selection.item, kind,
                          before.selection.target, 419, -1, 0, 3, True, True, True, False)
    return Record(plan, 'placed', 'none', before.expected_after(), insertion).raw


def fields(raw):
    r = Record.decode(raw)
    p = r.insertion
    x, y, z = p.pos
    building = dict(id=p.building, native=40 + p.kind, kind=('', 'Bed', 'Chair', 'Table')[p.kind],
                    bounds=(x, y, x, y, z), stage=3, maximum=3)
    item = dict(id=p.item, native=r.plan.before.item.native_type, kind=('', 'BED', 'CHAIR', 'TABLE')[p.kind],
                subtype=-1, material=419, material_index=-1, stack=1, position=p.pos,
                flags=256, container=None, holder=p.building)
    return building, item


def job(identity=100, *, kind='ConstructBuilding', holder=200, attachments=0, suspended=False):
    return dict(id=identity, native={'ConstructBuilding': 1, 'DestroyBuilding': 2}.get(kind, 3),
                kind=kind, holder=holder, attachments=attachments, suspended=suspended)


def operations(*, records=None, buildings=None, items=None, jobs=(), attachments=(), tick=20,
               paused=True, site=2, folder='region1', horizons=(1000, 2000, 3000)):
    """Independent DFMO1400/DFMJ1200 encoder; NOT the implementation's codec."""
    records = (receipt(),) if records is None else records
    pairs = [fields(r) for r in records]
    buildings = [b for b, _ in pairs] if buildings is None else buildings
    items = [i for _, i in pairs] if items is None else items
    u = lambda n: struct.pack('>I', n)
    i = lambda n: struct.pack('>i', n)
    text = lambda s: struct.pack('>H', len(s.encode())) + s.encode()
    ref = lambda v: b'\0' if v is None else b'\1' + u(v)
    identity = lambda v: u(v['id']) + i(v['native']) + text(v['kind'])
    year, year_tick = divmod(tick, 403200)
    j = (b'DFMJ1200' + u(year) + u(year_tick) + bytes([paused]) + i(site) + u(horizons[0])
         + text(folder) + u(len(jobs)))
    for value in sorted(jobs, key=lambda v: v['id']):
        j += (identity(value) + text('') + bytes([value['suspended'], 0]) + i(2) + i(2) + i(5)
              + ref(None) + ref(value['holder']) + i(-1) + u(value['attachments']) + u(0))
    raw = b'DFMO1400' + u(len(j)) + j + u(horizons[1]) + u(horizons[2]) + u(len(buildings))
    for b in sorted(buildings, key=lambda v: v['id']):
        raw += identity(b) + b''.join(i(n) for n in (*b['bounds'], b['stage'], b['maximum']))
    raw += u(len(items))
    for v in sorted(items, key=lambda v: v['id']):
        raw += (identity(v) + i(v['subtype']) + i(v['material']) + i(v['material_index'])
                + u(v['stack']) + b''.join(i(n) for n in v['position']) + u(v['flags'])
                + ref(v['container']) + ref(v['holder']))
    raw += u(len(attachments)) + b''.join(i(n) for row in sorted(attachments) for n in row)
    return raw


class ConstructionPrefixTests(unittest.TestCase):
    def run_prefix(self, *, records=None, raw=None, **kwargs):
        records = (receipt(),) if records is None else records
        return assess_prefix(records, operations(records=records) if raw is None else raw, idle, **kwargs)

    def test_all_three_furniture_kinds_require_exact_installed_identity(self):
        for kind in (1, 2, 3):
            out = self.run_prefix(records=(receipt(kind=kind),))
            self.assertEqual(out['status'], 'quiescent')
            self.assertTrue(out['predecessor_gate_met'])
            self.assertEqual(out['rows'][0]['status'], 'quiescent')
            for name in ('goal_completion_proven', 'placement_authorized', 'retry_permitted',
                         'production_admitted', 'global_quiescence_proven', 'placement_effect_discharged',
                         'original_prefix_completeness_authenticated', 'native_acquisition_proven'):
                self.assertIs(out[name], False)

    def test_registered_job_and_stage_zero_do_not_unlock_a_successor(self):
        b, item = fields(receipt())
        b['stage'] = 0
        item.update(position=(1, 1, 5), flags=128, holder=None)
        out = self.run_prefix(raw=operations(buildings=[b], items=[item], jobs=[job(attachments=1)],
                                             attachments=[(100, 10, 0, -1)]))
        self.assertEqual(out['status'], 'pending')
        self.assertFalse(out['predecessor_gate_met'])
        self.assertEqual(out['rows'][0]['assessment']['construction_jobs'], 1)

    def test_suspended_construction_is_pending_not_complete(self):
        out = self.run_prefix(raw=operations(jobs=[job(suspended=True)]))
        self.assertEqual(out['rows'][0]['reason'], 'suspended')
        self.assertFalse(out['predecessor_gate_met'])

    def test_job_disappearance_alone_is_insufficient(self):
        b, item = fields(receipt())
        b['stage'] = 1
        out = self.run_prefix(raw=operations(buildings=[b], items=[item]))
        self.assertEqual(out['rows'][0]['reason'], 'no_construction_job')
        self.assertFalse(out['predecessor_gate_met'])

    def test_removal_and_unknown_held_jobs_block_even_a_complete_building(self):
        for kind, status, reason in [('DestroyBuilding', 'blocked', 'removal_pending'),
                                     ('FutureJob', 'pending', 'other_held_job')]:
            out = self.run_prefix(raw=operations(jobs=[job(101, kind=kind)]))
            self.assertEqual(out['status'], status)
            self.assertEqual(out['rows'][0]['reason'], reason)
            self.assertFalse(out['predecessor_gate_met'])

    def test_installed_original_item_must_be_at_the_original_target(self):
        b, item = fields(receipt())
        item['position'] = (9, 9, 5)
        out = self.run_prefix(raw=operations(buildings=[b], items=[item]))
        self.assertEqual(out['rows'][0]['reason'], 'installed_item_position_mismatch')
        self.assertFalse(out['predecessor_gate_met'])

    def test_item_attachments_and_original_job_id_reuse_are_not_quiescent(self):
        raw = operations(jobs=[job(101, kind='FutureJob', holder=None, attachments=1)],
                         attachments=[(101, 10, 0, -1)])
        self.assertEqual(self.run_prefix(raw=raw)['rows'][0]['reason'], 'item_unverified')
        raw = operations(jobs=[job(100, kind='FutureJob', holder=None)])
        self.assertEqual(self.run_prefix(raw=raw)['rows'][0]['reason'], 'original_job_identity_mismatch')

    def test_unrelated_job_does_not_make_selected_quiescence_global(self):
        out = self.run_prefix(raw=operations(jobs=[job(101, kind='FutureJob', holder=None)]))
        self.assertTrue(out['predecessor_gate_met'])
        self.assertFalse(out['global_quiescence_proven'])

    def test_missing_building_or_item_stays_unestablished(self):
        b, item = fields(receipt())
        for raw, reason in [(operations(buildings=[], items=[]), 'building_missing'),
                            (operations(buildings=[b], items=[]), 'item_missing')]:
            out = self.run_prefix(raw=raw)
            self.assertEqual(out['status'], 'unestablished')
            self.assertEqual(out['rows'][0]['reason'], reason)

    def test_complete_prefix_is_checked_in_one_shared_capture(self):
        records = tuple(receipt(i, kind=1 + i % 3) for i in range(32))
        pairs = [fields(raw) for raw in records]
        buildings = [b for b, _ in pairs]
        items = [i for _, i in pairs]
        buildings[7]['stage'] = 0
        out = self.run_prefix(records=records, raw=operations(records=records, buildings=buildings, items=items))
        self.assertEqual(out['selected_count'], 32)
        self.assertEqual(out['counts'], dict(quiescent=31, pending=1, blocked=0, unestablished=0))
        self.assertFalse(out['predecessor_gate_met'])
        self.assertLess(len(str(out)), 64 * 1024)

    def test_empty_prefix_is_vacuous_not_empty_fortress_evidence(self):
        out = self.run_prefix(records=(), raw=operations(jobs=[job(101, kind='FutureJob', holder=None)]))
        self.assertEqual(out['status'], 'no_predecessors')
        self.assertTrue(out['predecessor_gate_met'])
        self.assertFalse(out['global_quiescence_proven'])
        with self.assertRaises(ValueError):
            self.run_prefix(records=(), raw=b'bad')

    def test_all_item_flag_words_and_legacy_goal_assessment(self):
        b, item = fields(receipt())
        goal = Goal(receipt(), 1000)
        forbidden = 2 | 8 | 64 | 128
        for flags in range(512):
            item['flags'] = flags
            raw = operations(buildings=[b], items=[item])
            observed = decode_operations(raw, idle)
            self.assertEqual(assess(goal, observed, idle), assess_placed_receipt(goal.receipt, observed, idle))
            self.assertEqual(self.run_prefix(raw=raw)['predecessor_gate_met'],
                             bool(flags & 256) and not bool(flags & forbidden))

    def test_field_identity_mutations_never_become_success(self):
        changes = [('kind', 'FutureFurniture'), ('subtype', 1), ('material', 420),
                   ('material_index', 1), ('native', 90), ('stack', 2), ('holder', None)]
        for field, value in changes:
            b, item = fields(receipt())
            item[field] = value
            self.assertFalse(self.run_prefix(raw=operations(buildings=[b], items=[item]))['predecessor_gate_met'])
        for field, value in [('kind', 'Chair'), ('bounds', (3, 2, 3, 2, 5)), ('maximum', 4)]:
            b, item = fields(receipt())
            b[field] = value
            self.assertFalse(self.run_prefix(raw=operations(buildings=[b], items=[item]))['predecessor_gate_met'])

    def test_world_clock_and_native_horizons_are_not_inferred(self):
        for kw in [dict(folder='other'), dict(site=3), dict(tick=9),
                   dict(horizons=(100, 2000, 3000))]:
            self.assertFalse(self.run_prefix(raw=operations(**kw))['predecessor_gate_met'])
        with self.assertRaises(ValueError):
            self.run_prefix(raw=operations(paused=False))

    def test_stateless_assessment_needs_no_invented_future_deadline(self):
        raw = receipt(tick=MAX_TICK)
        self.assertTrue(self.run_prefix(records=(raw,), raw=operations(records=(raw,), tick=MAX_TICK))['predecessor_gate_met'])

    def test_malformed_complete_roster_and_every_truncation_are_refused(self):
        raw = operations()
        for length in range(len(raw)):
            with self.assertRaises(ValueError):
                self.run_prefix(raw=raw[:length])
        for bad in [raw + b'x', operations(jobs=[job(attachments=1)]),
                    operations(attachments=[(100, 10, 0, -1)])]:
            with self.assertRaises(ValueError):
                self.run_prefix(raw=bad)

    def test_uncertain_receipt_cannot_become_a_completed_predecessor(self):
        record = Record.decode(receipt())
        for phase, reason in [('prepared', 'none'), ('indeterminate', 'native_failure'),
                              ('refused', 'stale'), ('cancelled', 'cancelled')]:
            raw = Record(record.plan, phase, reason).raw
            with self.assertRaises(ValueError):
                self.run_prefix(records=(raw,), raw=operations())
            with self.assertRaises(ValueError):
                assess_placed_receipt(raw, decode_operations(operations(), idle), idle)

    def test_duplicate_foreign_out_of_order_and_overlong_prefixes_refuse(self):
        foreign = Record.decode(receipt(1))
        before = replace(foreign.plan.before, generation=12)
        foreign = Record(Plan('foreign', before), 'placed', 'none', before.expected_after(), foreign.insertion).raw
        for records in [(receipt(), receipt()), (receipt(1), receipt()), (receipt(), foreign),
                        tuple(receipt(i) for i in range(33))]:
            with self.assertRaises(ValueError):
                self.run_prefix(records=records, raw=operations())

    def test_every_receipt_truncation_and_corruption_refused(self):
        raw = receipt()
        for n in range(len(raw)):
            for bad in [raw[:n], raw[:n] + bytes([raw[n] ^ 1]) + raw[n + 1:]]:
                with self.assertRaises(ValueError):
                    self.run_prefix(records=(bad,), raw=operations())

    def test_exact_budget_and_semantic_evidence_survive_narrowing(self):
        out = self.run_prefix()
        again = self.run_prefix(maximum_work=out['work_units'])
        self.assertEqual(out['evidence_digest'], again['evidence_digest'])
        with self.assertRaises(ValueError):
            self.run_prefix(maximum_work=out['work_units'] - 1)
        for bad in [0, -1, True, 20_000_001]:
            with self.assertRaises(ValueError):
                self.run_prefix(maximum_work=bad)

    def test_every_cooperative_abort_returns_no_partial_result(self):
        calls = []
        assess_prefix((receipt(),), operations(), lambda: calls.append(1))
        for stop in range(1, len(calls) + 1):
            count = 0
            def guard():
                nonlocal count
                count += 1
                if count == stop:
                    raise ValueError('stop')
            with self.assertRaises(ValueError):
                assess_prefix((receipt(),), operations(), guard)

    def test_final_publication_guard_cannot_be_omitted(self):
        from unittest.mock import patch
        import construction_prefix as core
        revoked = False
        original = core.canonical
        def encode(value):
            nonlocal revoked
            raw = original(value)
            if 'maximum_work' in value:
                revoked = True
            return raw
        def guard():
            if revoked:
                raise ValueError('authority revoked during final encoding')
        with patch.object(core, 'canonical', side_effect=encode), self.assertRaises(ValueError):
            core.assess_prefix((receipt(),), operations(), guard)

    def test_evidence_covers_entire_capture_and_every_original_receipt(self):
        a = self.run_prefix()
        b = self.run_prefix(raw=operations(tick=21))
        self.assertNotEqual(a['evidence_digest'], b['evidence_digest'])
        c = self.run_prefix(raw=operations(jobs=[job(101, kind='FutureJob', holder=None)]))
        self.assertNotEqual(a['evidence_digest'], c['evidence_digest'])
        record = Record.decode(receipt())
        changed = Record(Plan('different-key', record.plan.before), 'placed', 'none', record.after, record.insertion).raw
        self.assertNotEqual(a['evidence_digest'], self.run_prefix(records=(changed,))['evidence_digest'])
        self.assertEqual(a['capture_sha256'], hashlib.sha256(operations()).hexdigest())


if __name__ == '__main__':
    unittest.main()
