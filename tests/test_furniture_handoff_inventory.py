"""Complete paged inventory -> retained request -> reviewed placement workflow.

Runs the existing Python TCP pager and placement clients against a joined, explicit
protocol peer. This is not a real DFHack SDK/game qualification campaign.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import allocate_furniture
import furniture_batch as batch
import furniture_inventory as inventory
from construction_receipt import Manifest
from furniture_allocation import Candidate, Request, Slot
from furniture_handoff import Handoff, Selected
from furniture_plan import canonical
from test_furniture_handoff import handoff
from test_furniture_handoff_batch import Peer, advance, encoded_fields, receive


def operations_bytes(h, extras=0, *, omit=(), flags=64):
    """Independent encoding of the complete operations/1.4 inventory fixture."""
    def u32(value):
        return struct.pack('>I', value)
    def i32(value):
        return struct.pack('>i', value)
    def text(value):
        raw = value.encode()
        return struct.pack('>H', len(raw)) + raw
    year, tick = divmod(h.source.tick, 403200)
    jobs = (b'DFMJ1200' + u32(year) + u32(tick) + b'\x01' + i32(h.request.site)
            + u32(h.source.horizons[0]) + text(h.request.folder) + u32(0))
    rows = [r for r in h.selections if r.candidate.id not in omit]
    rows += [Selected('irrelevant', Candidate(1000 + i, 'bed', (30000, 30000, 2), (419, -1), -1), 0)
             for i in range(extras)]
    rows.sort(key=lambda r: r.candidate.id)
    horizon = max([h.source.horizons[2]] + [r.candidate.id + 1 for r in rows])
    out = (b'DFMO1400' + u32(len(jobs)) + jobs + u32(h.source.horizons[1])
           + u32(horizon) + u32(0) + u32(len(rows)))
    for row in rows:
        c = row.candidate
        out += (u32(c.id) + i32(row.native_type) + text(c.kind.upper()) + i32(c.subtype)
                + i32(c.material[0]) + i32(c.material[1]) + u32(1)
                + b''.join(i32(n) for n in c.position) + u32(flags) + b'\0\0')
    return out + u32(0)


class PrefixSocket:
    def __init__(self, conn, prefix):
        self.conn, self.prefix = conn, prefix
    def recv(self, count):
        if self.prefix:
            out, self.prefix = self.prefix[:count], self.prefix[count:]
            return out
        return self.conn.recv(count)
    def sendall(self, raw):
        return self.conn.sendall(raw)


class InventoryPeer(Peer):
    def __init__(self, h, extras=0):
        super().__init__(h)
        self.raw = operations_bytes(h, extras)
        self.ops_events, self.pages = [], []
        self.releases = 0
        self.fault = None
        self.after_release = None

    def inventory_environment(self):
        return {inventory.OPT_IN: '1', inventory.ENDPOINT: self.handoff.source.address,
                inventory.OPERATIONS_TOKEN: 'o' * 32}

    def serve(self, conn):
        greeting = receive(conn, 12)
        assert greeting == b'DFHack?\n' + struct.pack('<i', 1)
        # The native greeting must be returned before the client binds a method.
        conn.sendall(b'DFHack!\n' + struct.pack('<i', 1))
        first_header = receive(conn, 8)
        method, width = struct.unpack('<h2xi', first_header)
        assert method == 0 and width <= 2048
        first = receive(conn, width)
        binding = inventory.decode(first)
        if binding[4] == b'dfmcp_build_v1_19':
            # Reuse the unchanged placement peer; suppress its already-sent greeting.
            class PlacementSocket(PrefixSocket):
                sent_greeting = False
                def sendall(self, raw):
                    if not self.sent_greeting:
                        self.sent_greeting = True
                        assert raw == b'DFHack!\n' + struct.pack('<i', 1)
                        return
                    return super().sendall(raw)
            return super().serve(PlacementSocket(conn, greeting + first_header + first))
        methods = {}
        request = binding
        page_count = 0
        while not self.done.is_set():
            if method == 0:
                assert request[2] == b'dfmcp.operations.v1_4.Request'
                assert request[3] == b'dfmcp.operations.v1_4.Reply'
                assert request[4] == b'dfmcp_operations_v1_4'
                name = request[1].decode()
                assert name in ('Handshake', 'ReadObservation') and name not in methods.values()
                number = len(methods) + 2
                methods[number] = name
                self.ops_events.append('bind:' + name)
                response = {1: number}
            else:
                assert request[1] == b'o' * 32 and request[3] == 1 and request[4] == 4
                assert request[5] == request[6] == 4096 and request[7] == 65536
                assert request[8] == 16 * 1024 * 1024 and request[11] == 65536
                name = methods[method]
                self.ops_events.append(name)
                response = {1: 1, 2: 0, 3: request[2], 4: 1, 5: 4,
                    6: self.handoff.source.generation, 7: self.handoff.source.df_version.encode(),
                    8: self.handoff.source.dfhack_version.encode()}
                if name == 'ReadObservation':
                    if request[12] == 1:
                        self.releases += 1
                        assert request[9] == b'c' * 16 and request[10] == 0
                        if self.fault == 'release':
                            return
                        response[10] = b'c' * 16
                        if self.after_release:
                            self.after_release()
                    else:
                        assert request[12] == 0
                        offset = request[10]
                        assert request[9] == (b'' if offset == 0 else b'c' * 16)
                        assert offset == page_count * 65536
                        self.pages.append(offset)
                        part = self.raw[offset:offset + 65536]
                        response.update({9: part, 10: b'c' * 16, 11: offset, 12: len(self.raw),
                            13: hashlib.sha256(self.raw).digest(),
                            14: int(offset + len(part) == len(self.raw))})
                        if self.fault == 'digest':
                            response[13] = b'x' * 32
                        if self.fault == 'page-token' and page_count:
                            response[10] = b'd' * 16
                        if self.fault == 'generation' and page_count:
                            response[6] += 1
                        page_count += 1
                else:
                    assert name == 'Handshake'
            raw = encoded_fields(response)
            conn.sendall(struct.pack('<h2xi', -1, len(raw)) + raw)
            header = receive(conn, 8)
            method, width = struct.unpack('<h2xi', header)
            assert 0 <= width <= 2048
            request = inventory.decode(receive(conn, width))


def export(peer, request=None):
    with patch.dict(os.environ, peer.inventory_environment(), clear=True):
        return json.loads(inventory.run(request or peer.handoff.request, inventory.Authority.load(),
                                       inventory.Budget(10000), retain_constraints=True))


class InventoryHandoffTests(unittest.TestCase):
    def test_export_retains_exact_manifest_capture_and_selected_native_facts(self):
        with InventoryPeer(handoff(), extras=1800) as peer:
            out = export(peer)
            h = Handoff.from_json(out['result']['handoff'])
            self.assertEqual(out['profile'], 'furniture-allocation-handoff/1')
            self.assertEqual(h.plan(), peer.handoff.plan())
            self.assertEqual(h.request, peer.handoff.request)
            self.assertEqual(h.source.capture_sha256, hashlib.sha256(peer.raw).hexdigest())
            self.assertEqual(h.source.capture_bytes, len(peer.raw))
            self.assertEqual(h.source.address, peer.handoff.source.address)
            self.assertEqual(h.source.generation, 17)
            self.assertEqual(h.source.horizons, (100, 200, 2800))
            self.assertEqual(out['result']['handoff_digest'], h.digest)
            self.assertTrue(out['result']['capture_release_verified'])
            self.assertGreater(len(peer.pages), 1)
            self.assertEqual(peer.pages, list(range(0, len(peer.raw), 65536)))
            self.assertEqual(peer.releases, 1)
            self.assertEqual(peer.events, [])
            self.assertEqual([e for e in peer.ops_events if e.startswith('bind:')],
                             ['bind:Handshake', 'bind:ReadObservation'])
            self.assertNotIn('assignments', out['result'])
            self.assertNotIn('request', out['result'])

    def test_existing_default_report_and_pure_projection_are_unchanged(self):
        with InventoryPeer(handoff()) as peer, patch.dict(os.environ, peer.inventory_environment(), clear=True):
            out = json.loads(inventory.run(peer.handoff.request, inventory.Authority.load(), inventory.Budget(10000)))
            projected = inventory.project(peer.handoff.request, peer.raw, lambda: None)
            self.assertEqual(out['profile'], 'furniture-allocation/1')
            self.assertNotIn('handoff', out['result'])
            self.assertEqual(out['result']['assignments'], projected['assignments'])
            self.assertEqual(out['result']['request'], peer.handoff.request.json())
            self.assertEqual(out['result']['plan'], projected['plan'])

    def test_shortage_has_no_executable_handoff_or_partial_batch(self):
        with InventoryPeer(handoff()) as peer, tempfile.TemporaryDirectory() as directory:
            peer.raw = operations_bytes(peer.handoff, omit=(41,))
            out = export(peer)
            self.assertEqual(out['result']['status'], 'shortage')
            self.assertIsNone(out['result']['handoff'])
            self.assertIsNone(out['result']['plan'])
            self.assertEqual(out['result']['assignments'], [])
            self.assertEqual(out['result']['request'], peer.handoff.request.json())
            self.assertEqual(os.listdir(directory), [])
            self.assertEqual(peer.events, [])
            self.assertEqual(peer.releases, 1)

    def test_bad_pages_digest_source_or_release_publish_no_handoff(self):
        for fault in ('digest', 'page-token', 'generation', 'release'):
            with self.subTest(fault=fault), InventoryPeer(handoff(), extras=1800) as peer:
                peer.fault = fault
                with self.assertRaises(ValueError):
                    export(peer)
                self.assertEqual(peer.events, [])
                self.assertLessEqual(peer.releases, 1)

    def test_full_decoder_rejects_malformed_roster_even_after_successful_release(self):
        with InventoryPeer(handoff()) as peer:
            peer.raw = peer.raw[:-1]
            with self.assertRaises(ValueError):
                export(peer)
            self.assertEqual(peer.releases, 1)
            self.assertEqual(peer.events, [])
        with InventoryPeer(handoff()) as peer:
            peer.raw = peer.raw.replace(b'DFMO1400', b'DFMO1300', 1)
            with self.assertRaises(ValueError):
                export(peer)

    def test_shared_budget_and_revocation_prevent_cached_export(self):
        with InventoryPeer(handoff()) as peer, patch.dict(os.environ, peer.inventory_environment(), clear=True):
            budget = inventory.Budget(10000)
            budget.work_steps = 1
            with self.assertRaises(ValueError):
                inventory.run(peer.handoff.request, inventory.Authority.load(), budget, retain_constraints=True)
            self.assertEqual(peer.releases, 1)
        with InventoryPeer(handoff()) as peer:
            original = inventory.bounded_output
            def revoked(*args, **kwargs):
                raw = original(*args, **kwargs)
                os.environ[inventory.OPT_IN] = '0'
                return raw
            with patch.object(inventory, 'bounded_output', revoked), self.assertRaises(ValueError):
                export(peer)
            self.assertEqual(peer.releases, 1)

    def test_no_union_of_read_and_placement_authority_is_accepted(self):
        with InventoryPeer(handoff()) as peer:
            for environment, load in (({**peer.inventory_environment(), 'DFMCP_BUILD_TOKEN': 't' * 32}, inventory.Authority.load),
                                      ({**peer.environment(), inventory.OPERATIONS_TOKEN: 'o' * 32}, batch.Authority.load)):
                with patch.dict(os.environ, environment, clear=True), self.assertRaises(ValueError):
                    load()
            self.assertEqual(peer.events, [])
            self.assertEqual(peer.ops_events, [])

    def test_export_option_is_validated_before_native_contact(self):
        with InventoryPeer(handoff()) as peer, patch.dict(os.environ, peer.inventory_environment(), clear=True):
            for bad in (1, 'true', None):
                with self.assertRaises(ValueError):
                    inventory.run(peer.handoff.request, inventory.Authority.load(), inventory.Budget(10000),
                                  retain_constraints=bad)
            self.assertEqual(peer.ops_events, [])

    def test_cli_export_init_restart_review_and_each_original_placement(self):
        with InventoryPeer(handoff(), extras=1800) as peer, tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            private = root / 'batch'
            private.mkdir(mode=0o700)
            request_file = root / 'request.json'
            request_file.write_bytes(canonical(peer.handoff.request.json()))
            run = subprocess.run([sys.executable, allocate_furniture.__file__, '--request-file',
                str(request_file), '--with-handoff'], capture_output=True, timeout=15,
                env=peer.inventory_environment())
            self.assertEqual(run.returncode, 0, run.stderr.decode())
            out = json.loads(run.stdout)
            artifact = root / 'handoff.json'
            artifact.write_bytes(canonical(out['result']['handoff']))
            run = subprocess.run([sys.executable, batch.__file__, 'init', '--directory', str(private),
                '--handoff', str(artifact), '--world-folder', peer.handoff.request.folder,
                '--site', str(peer.handoff.request.site)], capture_output=True, timeout=15,
                env=peer.environment())
            self.assertEqual(run.returncode, 0, run.stderr.decode())
            result = json.loads(run.stdout)['result']
            artifact.unlink()  # Restart never needs the original input pathname again.
            self.assertEqual(peer.commits, 0)
            for expected in (1, 2):
                review = subprocess.run([sys.executable, batch.__file__, 'review', '--directory', str(private),
                    '--batch-id', result['batch_id']], capture_output=True, timeout=15, env=peer.environment())
                self.assertEqual(review.returncode, 0, review.stderr.decode())
                offer = json.loads(review.stdout)['result']
                acted = subprocess.run([sys.executable, batch.__file__, 'advance', '--directory', str(private),
                    '--batch-id', result['batch_id'], '--expected-plan', offer['expected_plan'],
                    '--confirm-review', offer['confirm_review']], capture_output=True, timeout=15, env=peer.environment())
                self.assertEqual(acted.returncode, 0, acted.stderr.decode())
                result = json.loads(acted.stdout)['result']
                self.assertEqual(result['placed'], expected)
            self.assertEqual(result['status'], 'all_placed')
            self.assertEqual(peer.commits, 2)
            self.assertEqual(peer.releases, 1)
            self.assertFalse(result['construction_completion_proven'])
            self.assertEqual(result['allocation']['handoff_digest'], out['result']['handoff_digest'])

    def test_32_slot_paged_inventory_export_fits_complete_output_and_batch_custody(self):
        names = tuple(f'{i:02}-' + 'x' * 30 for i in range(32))
        request = Request('r' * 512, 2**31 - 1, tuple(Slot(name, 'bed', (i + 2, 15, 2),
            after=names[max(0, i - 7):i], max_distance=10) for i, name in enumerate(names)))
        selected = tuple(Selected(s.name, Candidate(i + 1, 'bed', s.target, (419, -1), -1), 0)
                         for i, s in enumerate(request.slots))
        h = Handoff(request, replace(handoff().source, df_version='v' * 128, dfhack_version='h' * 128), selected)
        with InventoryPeer(h, extras=2000) as peer, tempfile.TemporaryDirectory() as directory:
            out = export(peer)
            retained = Handoff.from_json(out['result']['handoff'])
            self.assertEqual(retained.plan(), h.plan())
            self.assertLessEqual(len(canonical(out)), 65536)
            self.assertGreater(len(peer.pages), 1)
            with patch.dict(os.environ, peer.environment(), clear=True):
                result = batch.initialize(directory, retained.plan(), request.folder, request.site, handoff=retained)
                for i in range(32):
                    result = advance(directory, result['batch_id'])
                    self.assertEqual(result['placed'], i + 1)
                inspected = batch.inspect(directory, result['batch_id'], allocation=True)
            self.assertEqual(peer.releases, 1)
            self.assertEqual(peer.commits, 32)
            self.assertEqual(inspected['allocation'], retained.json())
            self.assertLessEqual(len(batch.encoded('inspect', inspected)), 65536)
            print('32-slot export bytes:', len(canonical(out)), 'native pages:', len(peer.pages),
                  'full inspection bytes:', len(batch.encoded('inspect', inspected)))


if __name__ == '__main__':
    unittest.main()
