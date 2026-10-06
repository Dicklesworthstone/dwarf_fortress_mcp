"""Private-journal + joined map/1.5 + real CLI integration regressions.

These use synthetic native-format fixtures, not DFHack or a live fortress.
Unlike test_room_furniture_export_unit, these require the complete repository.
"""
from pathlib import Path
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

import excavation_observer as e
import export_room_furniture as x
from furniture_allocation import Request
import room_terrain_fixtures as f
import room_terrain_goal as r
from room_terrain_peer import Peer, TOKEN
import track_excavation as t

ROOT = Path(__file__).resolve().parents[1]


def capture(goal, tick, overrides=None, **kwargs):
    floors = (r.terrain.selection(goal.room_plan).floors if type(goal) is r.RoomTerrainGoal
              else f.coordinates(goal.region))
    tiles = {p: f.visible(3) for p in floors}
    tiles.update(overrides or {})
    raw = f.capture(goal.region, tiles, tick=tick, **kwargs)
    return e.decode_capture(raw, f.MANIFEST, goal.region)


def environment(address):
    return {**{k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')},
            'DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5': '1',
            'DFMCP_MAP_TOKEN': TOKEN.decode(), 'DFMCP_MAP_ENDPOINT': address}


def journal_bytes(goal, address, status='satisfied'):
    overrides = {}
    if status in ('pending', 'unknown'):
        floor = min(r.terrain.selection(goal.room_plan).floors)
        overrides[floor] = f.visible(2) if status == 'pending' else b'\x01'
    begin = {'kind': 'begin', 'format': t.profile_for_goal(goal).format,
             'nonce': 'a' * 64, 'endpoint': address, 'goal': goal.json(),
             'sample': t.sample_value(capture(goal, 100, overrides))}
    events = [begin]
    if status == 'cancelled':
        events.append({'kind': 'cancel'})
    elif status == 'pending_read':
        events.append({'kind': 'read_started'})
    elif status != 'stabilizing':
        latest = capture(goal, goal.deadline_tick + 1 if status == 'expired' else 110,
                         overrides, **({'folder': 'other'} if status == 'invalidated' else {}))
        events += [{'kind': 'read_started'}, {'kind': 'sample', 'sample': t.sample_value(latest)}]
    parts, head = [], '0' * 64
    for index, event in enumerate(events):
        raw = t.frame(event, index, head, t.profile_for_goal(goal))
        parts.append(raw)
        head = json.loads(raw)['sha256']
    return b''.join(parts)


class RoomFurnitureExportTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = Path(self.directory.name).resolve() / 'room-goal.journal'
        os.chmod(self.path.parent, 0o700)
        self.goal = r.RoomTerrainGoal(f.plan(), 1000)

    def tearDown(self):
        self.directory.cleanup()

    def seed(self, address, status='satisfied', goal=None):
        raw = journal_bytes(goal or self.goal, address, status)
        self.path.write_bytes(raw)
        os.chmod(self.path, 0o600)
        return raw

    def test_replayed_whole_goal_and_fresh_peer_produce_exact_existing_request(self):
        fresh = capture(self.goal, 115)
        with Peer(fresh.raw, self.goal.region) as peer:
            original = self.seed(peer.address)
            with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                result = json.loads(x.export(self.path))
            self.assertEqual((peer.accepted, peer.handshakes, peer.observations), (1, 1, 1))
            self.assertEqual(peer.binds, ['Handshake', 'ReadObservation'])
            self.assertGreater(peer.fragments, 4)
        request = Request.from_json(result['furniture_request'])
        self.assertEqual(request.json(), self.goal.room_plan.request().json())
        self.assertEqual(result['origin']['furniture_request_digest'], request.digest)
        self.assertEqual(result['room_goal'], self.goal.json())
        self.assertEqual(result['fresh_sample'], t.sample_value(fresh))
        self.assertEqual(result['completion_to_export_gap_ticks'], 5)
        self.assertEqual(self.path.read_bytes(), original)
        for field in ('items_allocated', 'journal_written', 'game_mutations_dispatched',
                      'current_conditions_proven', 'room_completion_proven',
                      'furniture_placement_eligibility_proven', 'production_admitted'):
            self.assertIs(result[field], False)

    def test_each_unsatisfied_terminal_or_interrupted_goal_refuses_before_connect(self):
        fresh = capture(self.goal, 115)
        for status in ('pending', 'unknown', 'stabilizing', 'pending_read', 'cancelled', 'expired', 'invalidated'):
            with self.subTest(status=status), Peer(fresh.raw, self.goal.region) as peer:
                original = self.seed(peer.address, status)
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises(ValueError, x.export, self.path, emit='request')
                self.assertEqual(peer.accepted, 0)
                self.assertEqual(self.path.read_bytes(), original)

    def test_legacy_floor_and_blueprint_goals_cannot_substitute_for_whole_room(self):
        region = e.Region((10, 10, 2), (1, 1, 1))
        blueprint = t.b.Blueprint.decode(e.canonical({'schema': 'dfmcp.excavation-blueprint/1',
            'parts': [{'region': region.json(), 'shape': 'floor'}]}))
        goals = [e.Goal(region, 'region1', 2, 1000), t.b.BlueprintGoal(blueprint, 'region1', 2, 1000)]
        for goal in goals:
            with self.subTest(goal=type(goal).__name__), Peer(capture(goal, 115).raw, goal.region) as peer:
                original = self.seed(peer.address, goal=goal)
                self.assertEqual(t.replay(original).progress.status, 'satisfied')
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises(ValueError, x.export, self.path)
                self.assertEqual(peer.accepted, 0)
                self.assertEqual(self.path.read_bytes(), original)

    def test_loss_of_any_original_wall_or_floor_refuses_no_partial_request(self):
        selected = r.terrain.selection(self.goal.room_plan)
        variants = [(min(selected.walls), f.visible(3)), (min(selected.floors), f.visible(2)),
                    (min(selected.walls), b'\x01'), (min(selected.floors), f.visible(3, depth=1))]
        for point, tile in variants:
            fresh = capture(self.goal, 115, {point: tile})
            with self.subTest(point=point, tile=tile), Peer(fresh.raw, self.goal.region) as peer:
                original = self.seed(peer.address)
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises(ValueError, x.export, self.path, emit='request')
                self.assertEqual(peer.observations, 1)
                self.assertEqual(self.path.read_bytes(), original)

    def test_target_occupancy_blocks_but_non_target_floor_occupancy_does_not(self):
        target = self.goal.room_plan.request().slots[-1].target
        selected = r.terrain.selection(self.goal.room_plan)
        other = min(selected.floors - {s.target for s in self.goal.room_plan.request().slots})
        for point, attributes, accepted in ((target, {'building': 1}, False),
                                             (target, {'units': 1}, False),
                                             (other, {'units': 1}, True)):
            fresh = capture(self.goal, 115, {point: f.visible(3, **attributes)})
            with self.subTest(point=point, attributes=attributes), Peer(fresh.raw, self.goal.region) as peer:
                original = self.seed(peer.address)
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    if accepted:
                        self.assertEqual(Request.decode(x.export(self.path, emit='request')).json(),
                                         self.goal.room_plan.request().json())
                    else:
                        self.assertRaises(ValueError, x.export, self.path)
                self.assertEqual(self.path.read_bytes(), original)

    def test_source_changes_and_clock_gap_refuse_without_rewriting_terminal_history(self):
        variants = [(capture(self.goal, 109), f.MANIFEST),
                    (capture(self.goal, 110 + self.goal.max_gap_ticks + 1), f.MANIFEST),
                    (capture(self.goal, 115, folder='other'), f.MANIFEST),
                    (capture(self.goal, 115, site=3), f.MANIFEST),
                    (capture(self.goal, 115, dimensions=(100, 100, 10)), f.MANIFEST),
                    (capture(self.goal, 115), e.Manifest(8, 'test-df', 'test-dfhack')),
                    (capture(self.goal, 115), e.Manifest(7, 'changed', 'test-dfhack'))]
        for fresh, manifest in variants:
            with self.subTest(tick=fresh.tick, manifest=manifest), Peer(fresh.raw, self.goal.region, manifest=manifest) as peer:
                original = self.seed(peer.address)
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises(ValueError, x.export, self.path)
                self.assertEqual(self.path.read_bytes(), original)

    def test_native_failures_never_reread_or_publish_original_request_as_fallback(self):
        fresh = capture(self.goal, 115)
        for fault in ('nonce', 'generation', 'lost_reply', 'truncated_reply', 'duplicate_field'):
            with self.subTest(fault=fault), Peer(fresh.raw, self.goal.region, fault=fault) as peer:
                original = self.seed(peer.address)
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises((ValueError, OSError, EOFError), x.export, self.path, emit='request')
                self.assertEqual(peer.accepted, 1)
                self.assertEqual(peer.observations, 1)
                self.assertEqual(self.path.read_bytes(), original)

    def test_missing_torn_and_replaced_journal_custody_fail_closed(self):
        fresh = capture(self.goal, 115)
        for kind in ('missing', 'torn', 'replaced'):
            def replace_file():
                old = self.path.with_name('preserved-original.journal')
                self.path.rename(old)
                self.path.write_bytes(old.read_bytes())
                os.chmod(self.path, 0o600)
            with self.subTest(kind=kind), Peer(fresh.raw, self.goal.region,
                    on_observation=replace_file if kind == 'replaced' else None) as peer:
                original = self.seed(peer.address)
                selected = self.path.with_name('absent.journal') if kind == 'missing' else self.path
                if kind == 'torn':
                    self.path.write_bytes(original[:-1])
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises((ValueError, OSError), x.export, selected)
                self.assertEqual(peer.accepted, 1 if kind == 'replaced' else 0)
                if kind == 'replaced':
                    self.assertEqual(self.path.with_name('preserved-original.journal').read_bytes(), original)

    def test_revocation_after_read_or_after_serialization_cannot_publish(self):
        fresh = capture(self.goal, 115)
        def revoke():
            os.environ.pop('DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5', None)
        original_select = x._select_output
        def serialized(*args):
            result = original_select(*args)
            revoke()
            return result
        for phase in ('read', 'serialization'):
            with self.subTest(phase=phase), Peer(fresh.raw, self.goal.region,
                    on_observation=revoke if phase == 'read' else None) as peer:
                original = self.seed(peer.address)
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    with mock.patch.object(x, '_select_output', serialized if phase == 'serialization' else original_select):
                        self.assertRaises((ValueError, OSError), x.export, self.path)
                self.assertEqual(self.path.read_bytes(), original)

    def test_original_work_deadline_and_four_call_allowance_are_not_renewed(self):
        fresh = capture(self.goal, 115)
        for kind in ('work', 'deadline', 'calls'):
            with self.subTest(kind=kind), Peer(fresh.raw, self.goal.region) as peer:
                original = self.seed(peer.address)
                budget = t.Budget(10000)
                if kind == 'work':
                    budget.work_left = 0
                elif kind == 'deadline':
                    budget.deadline = time.monotonic() - 1
                else:
                    budget.calls_left = 3
                with mock.patch.dict(os.environ, environment(peer.address), clear=True):
                    self.assertRaises(ValueError, x.export, self.path, _budget=budget)
                self.assertEqual(peer.observations, 0)
                self.assertEqual(self.path.read_bytes(), original)

    def test_real_cli_exports_32_slots_with_original_constraints_and_exclusions(self):
        intent = f.request(f.dining(16, 4))
        intent['excluded_items'] = list(range(100, 746))
        intent['areas'][0]['item_constraints'] = {
            kind: {'material': [0, -1], 'subtype': -1, 'max_distance': 400}
            for kind in ('chair', 'table')}
        goal = r.RoomTerrainGoal(f.RoomPlan.compile(intent), 1000)
        fresh = capture(goal, 115)
        with Peer(fresh.raw, goal.region) as peer:
            original = self.seed(peer.address, goal=goal)
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/export_room_furniture.py'),
                '--journal', str(self.path), '--emit', 'request'], env=environment(peer.address),
                capture_output=True, timeout=20, check=False)
            self.assertEqual(result.returncode, 0, result.stderr.decode() + result.stdout.decode())
            self.assertEqual(peer.observations, 1)
        self.assertEqual(result.stdout, e.canonical(goal.room_plan.request().json()))
        restored = Request.decode(result.stdout)
        self.assertEqual(len(restored.slots), 32)
        self.assertEqual(restored.excluded_items, tuple(range(100, 746)))
        self.assertEqual(self.path.read_bytes(), original)

    def test_short_stdout_is_not_retried_and_no_second_object_is_written(self):
        fresh = capture(self.goal, 115)
        output = mock.Mock()
        output.buffer.write.return_value = 0
        with Peer(fresh.raw, self.goal.region) as peer:
            original = self.seed(peer.address)
            with mock.patch.dict(os.environ, environment(peer.address), clear=True), mock.patch.object(sys, 'stdout', output):
                code = x.main(['--journal', str(self.path), '--emit', 'request'])
            self.assertEqual(code, 2)
            output.buffer.write.assert_called_once()
            self.assertEqual(peer.observations, 1)
            self.assertEqual(self.path.read_bytes(), original)


if __name__ == '__main__':
    unittest.main()
