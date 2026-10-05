"""Whole-original-plan completion retains allocation constraints and source identity.

Real TCP placements and original POSIX custody feed the actual completion codecs
and evaluator. Later construction observations here are explicit wire fixtures.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import furniture_batch as batch
import furniture_completion as completion
import construction_plan as condition
import build_placement_rpc as rpc
from build_placement_wire import Record
from construction_receipt import Manifest
from furniture_allocation import Candidate, Request, Slot
from furniture_handoff import Handoff, Selected
from test_furniture_handoff import handoff
from test_furniture_handoff_batch import advance, initialize, setup
from test_furniture_handoff_inventory import InventoryPeer, export


def finished_capture(origin, tick, *, missing=None, wrong_material=None, incomplete=None, item_horizon=None):
    def u32(value):
        return struct.pack('>I', value)
    def i32(value):
        return struct.pack('>i', value)
    def string(value):
        raw = value.encode()
        return struct.pack('>H', len(raw)) + raw
    records = [Record.decode(raw) for raw in origin.receipts]
    first = records[0].plan.before
    year, day = divmod(tick, 403200)
    jobs = (b'DFMJ1200' + u32(year) + u32(day) + b'\0' + i32(first.site)
            + u32(max(r.after.next_job for r in records)) + string(first.folder) + u32(0))
    ih = max(r.insertion.item + 1 for r in records)
    if origin.handoff is not None:
        ih = max(ih, origin.handoff.source.horizons[2])
    if item_horizon is not None:
        ih = item_horizon
    buildings = [r for r in records if r.insertion.building != missing]
    out = (b'DFMO1400' + u32(len(jobs)) + jobs + u32(max(r.after.next_building for r in records))
           + u32(ih) + u32(len(buildings)))
    for record in sorted(buildings, key=lambda r: r.insertion.building):
        insertion = record.insertion
        x, y, z = insertion.pos
        stage = 0 if insertion.building == incomplete else insertion.max_stage
        out += (u32(insertion.building) + i32(insertion.kind)
                + string(('', 'Bed', 'Chair', 'Table')[insertion.kind])
                + b''.join(i32(n) for n in (x, y, x, y, z)) + i32(stage) + i32(insertion.max_stage))
    items = [r for r in records if r.insertion.building != missing]
    out += u32(len(items))
    for record in sorted(items, key=lambda r: r.insertion.item):
        before, p = record.plan.before.item, record.insertion
        material = before.material + 1 if p.item == wrong_material else before.material
        out += (u32(p.item) + i32(before.native_type) + string(('', 'BED', 'CHAIR', 'TABLE')[p.kind])
                + i32(before.subtype) + i32(material) + i32(before.material_index) + u32(1)
                + b''.join(i32(n) for n in p.pos) + u32(256) + b'\0\x01' + u32(p.building))
    return out + u32(0)


def sample(goal, tick, **kwargs):
    source = goal.origin.source
    build = Manifest(source.generation, source.df_version, source.dfhack_version)
    operations = Manifest(17 if goal.origin.handoff is None else goal.origin.handoff.source.generation,
                          source.df_version, source.dfhack_version)
    return condition.LinkedSample(build, goal.condition.receipts, operations,
        finished_capture(goal.origin, tick, **kwargs), build, goal.condition.receipts)


def placed_origin(directory, peer, legacy=False):
    h = peer.handoff
    result = (batch.initialize(directory, h.plan(), h.request.folder, h.request.site)
              if legacy else initialize(directory, peer))
    for _ in h.plan().ordered:
        result = advance(directory, result['batch_id'])
    with batch.Batch(directory, rpc.Budget(10000)) as owner:
        return completion.Origin.from_batch(owner, result['batch_id'])


def goal_from(origin):
    return completion.Goal(origin, condition.Goal(origin.receipts, 50100))


class HandoffCompletionTests(unittest.TestCase):
    def test_complete_original_allocation_survives_origin_and_goal_round_trips(self):
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer)
            self.assertEqual(origin.handoff, peer.handoff)
            self.assertEqual(origin.encode()[:8], completion.HANDOFF_ORIGIN_MAGIC)
            self.assertEqual(completion.Origin.decode(origin.encode()), origin)
            goal = goal_from(origin)
            self.assertEqual(goal.encode()[:8], completion.HANDOFF_GOAL_MAGIC)
            self.assertEqual(completion.Goal.decode(goal.encode()), goal)
            self.assertEqual(origin.digest, hashlib.sha256(
                b'dfmcp.furniture-completion-origin/2\0' + origin.encode()).hexdigest())
            self.assertEqual(goal.digest, hashlib.sha256(
                b'dfmcp.furniture-completion-goal/2\0' + goal.encode()).hexdigest())

    def test_v1_bytes_and_digest_domains_remain_unchanged(self):
        def field(value):
            return struct.pack('>H', len(value)) + value
        def blob(value):
            return struct.pack('>I', len(value)) + value
        def ident(value):
            return struct.pack('>QQ', *value)
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer, legacy=True)
            self.assertIsNone(origin.handoff)
            expected = (b'DFMFCO01' + field(origin.batch_path.encode()) + ident(origin.manifest_identity)
                + blob(origin.manifest_raw) + ident(origin.index_identity) + blob(origin.index_raw)
                + bytes([len(origin.children)]))
            for child in origin.children:
                expected += field(child.name.encode()) + ident(child.file_identity) + blob(child.raw)
            self.assertEqual(origin.encode(), expected)
            self.assertEqual(origin.digest, hashlib.sha256(b'dfmcp.furniture-completion-origin/1\0' + expected).hexdigest())
            goal = goal_from(origin)
            expected_goal = b'DFMFCG01' + blob(expected) + blob(goal.condition.encode())
            self.assertEqual(goal.encode(), expected_goal)
            self.assertEqual(goal.digest, hashlib.sha256(b'dfmcp.furniture-completion-goal/1\0' + expected_goal).hexdigest())
            self.assertEqual(completion.Goal.decode(expected_goal), goal)

    def test_version_confusion_and_header_downgrade_are_rejected(self):
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer)
            with self.assertRaises(ValueError):
                completion.Origin.decode(b'DFMFCO01' + origin.encode()[8:])
            goal = goal_from(origin)
            with self.assertRaises(ValueError):
                completion.Goal.decode(b'DFMFCG01' + goal.encode()[8:])
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer, legacy=True)
            with self.assertRaises(ValueError):
                completion.Origin.decode(b'DFMFCO02' + origin.encode()[8:])
            goal = goal_from(origin)
            with self.assertRaises(ValueError):
                completion.Goal.decode(b'DFMFCG02' + goal.encode()[8:])

    def test_all_targets_must_be_complete_together_with_distinct_stable_samples(self):
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer)
            goal = goal_from(origin)
            state = completion.Progress(goal.digest)
            state = completion.advance(completion.begin_read(state), goal, sample(goal, 50001), lambda: None)
            self.assertEqual(state.phase, 'candidate')
            paused = completion.advance(completion.begin_read(state), goal, sample(goal, 50001), lambda: None)
            self.assertEqual(paused.streak, 1)
            final = completion.advance(completion.begin_read(paused), goal, sample(goal, 50002), lambda: None)
            self.assertEqual(final.phase, 'satisfied')
            self.assertEqual(len(final.assessments), 2)
            self.assertFalse(final.view()['current_usability_proven'])
            self.assertFalse(final.view()['placement_effect_discharged'])
            self.assertFalse(final.view()['continuous_stability_proven'])
            self.assertEqual(peer.commits, 2)

    def test_partial_completion_does_not_latch_success_from_another_target(self):
        with setup() as (directory, peer):
            goal = goal_from(placed_origin(directory, peer))
            first = Record.decode(goal.receipts[0]).insertion.building
            second = Record.decode(goal.receipts[1]).insertion.building
            state = completion.Progress(goal.digest)
            state = completion.advance(completion.begin_read(state), goal,
                sample(goal, 50001, incomplete=first), lambda: None)
            self.assertEqual(state.phase, 'active')
            self.assertEqual(state.streak, 0)
            # Completing the first while the second remains incomplete still fails
            # a single same-capture whole-plan condition; no earlier success latches.
            other = completion.advance(completion.begin_read(completion.Progress(goal.digest)), goal,
                sample(goal, 50002, incomplete=second), lambda: None)
            self.assertEqual(other.phase, 'active')
            self.assertEqual(other.view()['condition_met_count'], 1)

    def test_operation_incarnation_and_allocation_item_horizon_cannot_regress(self):
        with setup() as (directory, peer):
            goal = goal_from(placed_origin(directory, peer))
            initial = completion.begin_read(completion.Progress(goal.digest))
            observation = sample(goal, 50001)
            wrong = replace(observation, operations=replace(observation.operations, generation=18))
            with self.assertRaises(ValueError):
                completion.advance(initial, goal, wrong, lambda: None)
            # The older item horizon is still sufficient for every selected ID,
            # but contradicts the complete original allocation capture.
            regressed = completion.advance(initial, goal, sample(goal, 50001, item_horizon=43), lambda: None)
            self.assertEqual(regressed.phase, 'invalidated')
            self.assertEqual(regressed.reason, 'allocation_source_regressed')

    def test_original_material_and_receipt_selection_remain_required(self):
        with setup() as (directory, peer):
            goal = goal_from(placed_origin(directory, peer))
            initial = completion.begin_read(completion.Progress(goal.digest))
            wrong = completion.advance(initial, goal, sample(goal, 50001, wrong_material=41), lambda: None)
            self.assertEqual(wrong.phase, 'invalidated')
            self.assertEqual(wrong.reason, 'item_identity_mismatch')
            with self.assertRaises(ValueError):
                completion.Goal(goal.origin, condition.Goal(goal.receipts[:1], goal.deadline))
            observation = sample(goal, 50001)
            with self.assertRaises(ValueError):
                completion.advance(initial, goal, replace(observation, after_records=observation.after_records[:1]), lambda: None)

    def test_original_handoff_is_validated_during_independent_origin_replay(self):
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer)
            value = batch.unseal(origin.manifest_raw)
            value['handoff']['request']['excluded_items'] = [41]
            with self.assertRaises(ValueError):
                replace(origin, manifest_raw=batch.seal(value))
            value = batch.unseal(origin.manifest_raw)
            value['handoff']['selections'][0]['candidate']['material'] = [888, -1]
            with self.assertRaises(ValueError):
                replace(origin, manifest_raw=batch.seal(value))

    def test_original_file_identity_required_and_stop_does_not_erase_completion(self):
        with setup() as (directory, peer):
            origin = placed_origin(directory, peer)
            batch.stop(directory, origin.batch_id)
            with batch.Batch(directory, rpc.Budget(10000)) as owner:
                origin.verify_batch(owner)
            path = Path(directory, 'batch.json')
            replacement = path.with_name('replacement')
            replacement.write_bytes(path.read_bytes())
            replacement.chmod(0o600)
            replacement.replace(path)
            with batch.Batch(directory, rpc.Budget(10000)) as owner, self.assertRaises(ValueError):
                origin.verify_batch(owner)

    def test_pending_placement_and_incomplete_registered_prefix_cannot_start_completion(self):
        with setup() as (directory, peer):
            result = initialize(directory, peer)
            with batch.Batch(directory, rpc.Budget(10000)) as owner, self.assertRaises(ValueError):
                completion.Origin.from_batch(owner, result['batch_id'])
            advance(directory, result['batch_id'])
            with batch.Batch(directory, rpc.Budget(10000)) as owner, self.assertRaises(ValueError):
                completion.Origin.from_batch(owner, result['batch_id'])

    def test_32_slot_native_inventory_and_placement_to_whole_plan_completion(self):
        names = tuple(f'{i:02}-' + 'x' * 30 for i in range(32))
        request = Request('r' * 512, 2**31 - 1, tuple(Slot(name, 'bed', (i + 2, 15, 2),
            after=names[max(0, i - 7):i], max_distance=10) for i, name in enumerate(names)))
        selected = tuple(Selected(s.name, Candidate(i + 1, 'bed', s.target, (419, -1), -1), 0)
                         for i, s in enumerate(request.slots))
        h = Handoff(request, replace(handoff().source, df_version='v' * 128, dfhack_version='h' * 128), selected)
        with InventoryPeer(h, extras=2000) as peer, tempfile.TemporaryDirectory() as directory:
            retained = Handoff.from_json(export(peer)['result']['handoff'])
            with patch.dict(os.environ, peer.environment(), clear=True):
                result = batch.initialize(directory, retained.plan(), request.folder, request.site, handoff=retained)
                for _ in range(32):
                    result = advance(directory, result['batch_id'])
                with batch.Batch(directory, rpc.Budget(10000)) as owner:
                    origin = completion.Origin.from_batch(owner, result['batch_id'])
                    origin.verify_batch(owner)
            decoded = completion.Origin.decode(origin.encode())
            self.assertEqual(decoded.handoff, retained)
            goal = completion.Goal.decode(goal_from(decoded).encode())
            state = completion.Progress(goal.digest)
            for tick in (50001, 50002):
                state = completion.advance(completion.begin_read(state), goal, sample(goal, tick), lambda: None)
            self.assertEqual(state.phase, 'satisfied')
            self.assertEqual(state.view()['condition_met_count'], 32)
            self.assertEqual(peer.commits, 32)
            self.assertEqual(peer.releases, 1)
            self.assertLessEqual(len(origin.encode()), completion.MAX_ORIGIN)
            print('32-slot completion origin bytes:', len(origin.encode()), 'targets:', len(state.assessments))


if __name__ == '__main__':
    unittest.main()
