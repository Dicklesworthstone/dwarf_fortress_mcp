"""Actual private terrain histories, raw map codecs, joined TCP and CLI tests.

Allocation input is generated locally, not acquired from a native inventory.
No DFHack/SDK/game, Rust/MCP or production admission is claimed.
"""
from contextlib import contextmanager
from dataclasses import replace
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import bind_room_terrain as bind
import excavation_observer as e
import room_terrain as terrain
import track_excavation as t
from furniture_plan import canonical
from room_furniture_handoff import RoomFurnitureHandoff
from room_provisioning import RoomPlan
from room_terrain_origin import TerrainOrigin, ReadBudget
from terrain_furniture_handoff import TerrainFurnitureHandoff, MAX_BYTES
from room_terrain_peer import Peer, TOKEN
import room_terrain_fixtures as f
from terrain_furniture_fixtures import capture, room_allocation, write_history


@contextmanager
def environment(address):
    env = {k: v for k, v in os.environ.items() if not k.startswith('DFMCP_')}
    env.update(DFMCP_ALLOW_UNADMITTED_EXCAVATION_V1_5='1', DFMCP_MAP_TOKEN=TOKEN.decode(),
               DFMCP_MAP_ENDPOINT=address)
    with patch.dict(os.environ, env, clear=True):
        yield


class TerrainHandoffTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.journal, self.input = self.root / 'terrain', self.root / 'allocation.json'
        self.plan = f.plan()

    def prepare(self, peer, plan=None, **kwargs):
        plan = plan or self.plan
        write_history(self.journal, plan, peer.address, **kwargs)
        room = room_allocation(plan, peer.address)
        self.input.write_bytes(room.encode())
        return room

    def artifact(self):
        with Peer(capture(self.plan).raw, terrain.selection(self.plan).region) as peer:
            room = self.prepare(peer)
            with environment(peer.address):
                raw = bind.bind(self.journal, self.input)
            self.assertEqual((peer.accepted, peer.handshakes, peer.observations), (1, 1, 1))
        return TerrainFurnitureHandoff.decode(raw), room

    def test_full_history_fresh_map_exact_allocation_and_offline_reopen(self):
        result, room = self.artifact()
        original = self.journal.read_bytes()
        self.input.unlink()
        self.assertEqual(result.room.encode(), room.encode())
        self.assertEqual(result.fresh.tick, 120)
        self.assertEqual(result.origin.json()['completed_tick'], 110)
        self.assertNotEqual(result.fresh.manifest.generation, room.allocation.source.generation)
        with patch('socket.socket', side_effect=AssertionError('offline origin contacted native')):
            with result.origin.open(self.plan, t.Budget(10000)) as owner:
                self.assertEqual(owner.raw, original)
        self.assertEqual(self.journal.read_bytes(), original)
        self.assertEqual(TerrainFurnitureHandoff.decode(result.encode()).encode(), result.encode())
        self.assertRaises(ValueError, RoomFurnitureHandoff.decode, result.encode())

    def test_real_cli_retains_32_slots_and_646_exclusions(self):
        request = f.request(f.dining(16, 4))
        request['excluded_items'] = list(range(10000, 10646))
        plan = RoomPlan.compile(request)
        observed = capture(plan)
        with Peer(observed.raw, observed.region) as peer:
            room = self.prepare(peer, plan)
            original = self.journal.read_bytes()
            with environment(peer.address):
                run = subprocess.run([sys.executable, str(Path(bind.__file__)), '--journal', str(self.journal),
                    '--room-handoff', str(self.input)], capture_output=True, timeout=20)
            self.assertEqual(run.returncode, 0, run.stdout)
            result = TerrainFurnitureHandoff.decode(run.stdout)
            self.assertEqual(result.room.encode(), room.encode())
            self.assertEqual(len(result.room.allocation.plan().steps), 32)
            self.assertEqual(result.room.room_plan.json()['intent']['excluded_items'], request['excluded_items'])
            self.assertEqual(self.journal.read_bytes(), original)
            self.assertLess(len(run.stdout), MAX_BYTES)
            self.assertEqual(peer.observations, 1)

    def test_incomplete_cancelled_or_wrong_profile_refused_before_network(self):
        for mode in ('pending', 'cancelled', 'floor'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / 'terrain'
                with Peer(capture(self.plan).raw, terrain.selection(self.plan).region) as peer:
                    if mode == 'floor':
                        region = e.Region((10, 10, 2), (1, 1, 1))
                        goal = e.Goal(region, 'region1', 2, 1000, 0, 1)
                        sample = e.decode_capture(f.capture(region, {(10, 10, 2): f.visible(3)}), f.MANIFEST, region)
                        with t.open_journal(path, t.Budget(10000), writable=True, create=True) as owner:
                            owner.append({'kind': 'begin', 'format': t.FLOOR_PROFILE.format, 'nonce': 'a' * 64,
                                'endpoint': peer.address, 'goal': goal.json(), 'sample': t.sample_value(sample)})
                    else:
                        write_history(path, self.plan, peer.address, ticks=(100,), cancel=mode == 'cancelled')
                    self.input.write_bytes(room_allocation(self.plan, peer.address).encode())
                    with environment(peer.address):
                        self.assertRaises(ValueError, bind.bind, path, self.input)
                    self.assertEqual(peer.accepted, 0)

    def test_same_furniture_but_changed_room_geometry_refused(self):
        old = f.plan(f.bedroom(1, (3, 3)))
        other = f.plan(f.bedroom(1, (3, 4)))
        self.assertEqual(old.request().json(), other.request().json())
        observed = capture(old)
        with Peer(observed.raw, observed.region) as peer:
            write_history(self.journal, old, peer.address)
            self.input.write_bytes(room_allocation(other, peer.address).encode())
            with environment(peer.address):
                self.assertRaises(ValueError, bind.bind, self.journal, self.input)
            self.assertEqual(peer.accepted, 0)

    def test_old_allocation_wrong_endpoint_or_software_refused_before_read(self):
        for field, value in (('tick', 109), ('address', '127.0.0.1:1'), ('df_version', 'changed')):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / 'terrain'
                with Peer(capture(self.plan).raw, terrain.selection(self.plan).region) as peer:
                    write_history(path, self.plan, peer.address)
                    room = room_allocation(self.plan, peer.address)
                    room = replace(room, allocation=replace(room.allocation, source=replace(room.allocation.source, **{field: value})))
                    self.input.write_bytes(room.encode())
                    with environment(peer.address):
                        self.assertRaises(ValueError, bind.bind, path, self.input)
                    self.assertEqual(peer.accepted, 0)

    def test_original_walls_floors_and_furniture_targets_are_not_dropped(self):
        selected = terrain.selection(self.plan)
        target = self.plan.request().slots[0].target
        changes = [(min(selected.walls), f.visible(3)), (min(selected.floors), f.visible(2)),
                   (target, f.visible(3, building=1)), (target, f.visible(3, units=1)),
                   (target, f.visible(3, depth=1)), (target, f.visible(3, magma=1)),
                   (target, f.visible(3, dig=1)), (target, b'\x01'), (target, b'\x00')]
        for point, tile in changes:
            with self.subTest(point=point, tile=tile), tempfile.TemporaryDirectory() as temp:
                observed = capture(self.plan, overrides={point: tile})
                with Peer(observed.raw, observed.region) as peer:
                    path = Path(temp) / 'terrain'
                    write_history(path, self.plan, peer.address)
                    self.input.write_bytes(room_allocation(self.plan, peer.address).encode())
                    with environment(peer.address):
                        self.assertRaises(ValueError, bind.bind, path, self.input)
                    self.assertEqual(peer.observations, 1)

    def test_non_target_occupancy_is_not_invented_placement_blocker(self):
        selected = terrain.selection(self.plan)
        point = min(selected.floors - {s.target for s in self.plan.request().slots})
        observed = capture(self.plan, overrides={point: f.visible(3, units=1)})
        with Peer(observed.raw, observed.region) as peer:
            self.prepare(peer)
            with environment(peer.address):
                TerrainFurnitureHandoff.decode(bind.bind(self.journal, self.input))

    def test_fresh_source_clock_and_gap_must_match(self):
        for kwargs in ({'tick': 114}, {'tick': 1311}, {'folder': 'other'}, {'site': 3},
                       {'dimensions': (100, 100, 100)}):
            with self.subTest(kwargs=kwargs), tempfile.TemporaryDirectory() as temp:
                observed = capture(self.plan, **kwargs)
                with Peer(observed.raw, observed.region) as peer:
                    path = Path(temp) / 'terrain'
                    write_history(path, self.plan, peer.address)
                    self.input.write_bytes(room_allocation(self.plan, peer.address).encode())
                    with environment(peer.address):
                        self.assertRaises(ValueError, bind.bind, path, self.input)

    def test_native_failures_never_fall_back_or_reconnect(self):
        for fault in ('lost_reply', 'nonce', 'generation', 'software', 'profile', 'duplicate_field', 'truncated_reply'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as temp:
                observed = capture(self.plan)
                with Peer(observed.raw, observed.region, fault=fault) as peer:
                    path = Path(temp) / 'terrain'
                    write_history(path, self.plan, peer.address)
                    original = path.read_bytes()
                    self.input.write_bytes(room_allocation(self.plan, peer.address).encode())
                    with environment(peer.address):
                        self.assertRaises(ValueError, bind.bind, path, self.input)
                    self.assertEqual(peer.accepted, 1)
                    self.assertEqual(path.read_bytes(), original)

    def test_custody_replaced_during_read_is_not_acknowledged(self):
        observed = capture(self.plan)
        def swap():
            original = self.journal.read_bytes()
            self.journal.rename(self.root / 'old')
            self.journal.write_bytes(original)
            self.journal.chmod(0o600)
        with Peer(observed.raw, observed.region, on_observation=swap) as peer:
            self.prepare(peer)
            with environment(peer.address):
                self.assertRaises(ValueError, bind.bind, self.journal, self.input)
            self.assertEqual(self.journal.read_bytes(), (self.root / 'old').read_bytes())

    def test_reference_without_original_history_is_not_a_proof(self):
        result, _ = self.artifact()
        value = result.json()
        value['terrain_origin']['journal_sha256'] = 'b' * 64
        forged = TerrainFurnitureHandoff.decode(canonical(value))
        with self.assertRaises(ValueError):
            with forged.origin.open(self.plan, t.Budget(10000)):
                pass
        self.journal.unlink()
        with self.assertRaises(OSError):
            with result.origin.open(self.plan, t.Budget(10000)):
                pass

    def test_raw_map_overrides_forged_in_process_fields(self):
        result, _ = self.artifact()
        forged = replace(result.fresh, tick=999, folder='forged', tiles=())
        self.assertEqual(replace(result, fresh=forged).fresh.tick, 120)
        self.assertRaises(ValueError, replace, result, fresh=replace(result.fresh, raw=result.fresh.raw + b'x'))

    def test_revocation_after_read_or_serialization_refuses(self):
        for phase in ('read', 'output'):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as temp:
                observed = capture(self.plan)
                revoke = lambda: os.environ.pop('DFMCP_MAP_TOKEN', None)
                with Peer(observed.raw, observed.region, on_observation=revoke if phase == 'read' else None) as peer:
                    path = Path(temp) / 'terrain'
                    write_history(path, self.plan, peer.address)
                    self.input.write_bytes(room_allocation(self.plan, peer.address).encode())
                    original = bind.output
                    def encode(value):
                        raw = original(value)
                        revoke()
                        return raw
                    with environment(peer.address), patch.object(bind, 'output', encode if phase == 'output' else original):
                        self.assertRaises(ValueError, bind.bind, path, self.input)
                    self.assertEqual(peer.observations, 1)

    def test_narrow_export_reserves_report_and_short_stdout_never_retries(self):
        observed = capture(self.plan)
        with Peer(observed.raw, observed.region) as peer:
            self.prepare(peer)
            writes = []
            class Short:
                @property
                def buffer(self):
                    return self
                def write(self, raw):
                    writes.append(raw)
                    return len(raw) - 1
            with environment(peer.address), patch.object(sys, 'stdout', Short()):
                code = bind.main(['--journal', str(self.journal), '--room-handoff', str(self.input)])
            self.assertEqual(code, 2)
            self.assertEqual(len(writes), 1)
            self.assertEqual(peer.observations, 1)
        self.journal.unlink()
        with Peer(observed.raw, observed.region) as peer:
            self.prepare(peer)
            with environment(peer.address), patch.object(bind, 'MAX_OUTPUT', 1):
                self.assertRaises(ValueError, bind.bind, self.journal, self.input)
            self.assertEqual(peer.observations, 1)

    def test_budget_adapters_do_not_renew_work_or_deadline(self):
        budget = t.Budget(10000)
        first, second = ReadBudget(budget), ReadBudget(budget)
        budget._terrain_origin_work_left = 1
        first.checkpoint()
        self.assertRaises(ValueError, second.checkpoint)
        budget = t.Budget(10000)
        original = budget.deadline
        adapter = ReadBudget(budget)
        self.assertEqual(budget.deadline, original)
        budget.deadline = 0
        self.assertRaises(ValueError, adapter.checkpoint)

    def test_closed_schemas_exact_numbers_nesting_and_hex(self):
        result, _ = self.artifact()
        variants = []
        for field, value in (('completed_tick', True), ('matching_samples', True), ('journal_bytes', 0),
                             ('journal_path', '/tmp/../source'), ('journal_sha256', 'X' * 64)):
            changed = result.json()
            changed['terrain_origin'][field] = value
            variants.append(canonical(changed))
        changed = result.json()
        changed['fresh_map']['capture_hex'] = changed['fresh_map']['capture_hex'].upper()
        variants.append(canonical(changed))
        variants += [result.encode() + b'\n', result.encode()[:-1] + b',"schema":"duplicate"}',
                     b'[' * 40 + b'0' + b']' * 40, b'x' * (MAX_BYTES + 1)]
        for raw in variants:
            self.assertRaises(ValueError, TerrainFurnitureHandoff.decode, raw)


if __name__ == '__main__':
    unittest.main()
