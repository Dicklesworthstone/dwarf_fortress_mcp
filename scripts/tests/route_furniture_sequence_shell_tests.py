"""Proposal/CLI unit tests with explicit mocked native-acquisition boundaries.

The actual request reader, map decoder, inventory projection, allocation,
sequencing, proposal packet and CLI run here. Native transport and operations
roster decoding are injected doubles, NOT evidence of native or live execution.
Private imports are isolated so this suite never replaces another suite's native
modules, opens a socket, or requires a running game.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from furniture_plan import canonical
import route_furniture_allocation as routes
from route_furniture_sequence_tests import corridor

SCRIPTS = Path(__file__).resolve().parents[1]


class BudgetDouble:
    """A shrinking explicit work allowance; wall time and transport are mocked."""
    def __init__(self, timeout_ms=10000, *, work_steps=routes.MAX_WORK):
        self.work_steps = work_steps
        self.disk_bytes = 1024 * 1024
        self.calls = 272
        self.network_bytes = 20 * 1024 * 1024

    def remaining(self):
        return 10.0

    def work(self):
        if self.work_steps <= 0:
            raise ValueError('work allowance exhausted')
        self.work_steps -= 1

    def reserve(self, name, amount):
        if not 0 <= amount <= getattr(self, name):
            raise ValueError('allowance exhausted')
        setattr(self, name, getattr(self, name) - amount)


@dataclass(frozen=True)
class ManifestDouble:
    generation: int
    df_version: str = 'synthetic-df'
    dfhack_version: str = 'synthetic-dfhack'

    @property
    def software(self):
        return self.df_version, self.dfhack_version


def load(name, path, imports):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: module, **imports}):
        spec.loader.exec_module(module)
    return module


def modules():
    wire = types.ModuleType('construction_monitor_rpc')
    wire.Client = object
    wire.Budget = BudgetDouble
    wire.OPERATIONS_TOKEN = 'DFMCP_OPERATIONS_PAGED_TOKEN'
    wire.PROFILES = {'operations': ('dfmcp_operations_v1_4', 'dfmcp.operations.v1_4', 4)}
    receipt = types.ModuleType('construction_receipt')
    receipt.Manifest = ManifestDouble
    receipt.decode_operations = lambda *args: (_ for _ in ()).throw(AssertionError('fixture not installed'))
    inventory = load('_sequence_inventory_test', SCRIPTS / 'route_furniture_inventory.py',
                     {'construction_monitor_rpc': wire, 'construction_receipt': receipt})
    cli = load('_sequence_cli_test', SCRIPTS / 'allocate_route_furniture.py',
               {'construction_monitor_rpc': wire, 'route_furniture_inventory': inventory})
    return inventory, cli


@contextmanager
def acquired(inventory, items, raw):
    """One synthetic acquisition; close-before-CPU is tested, not native paging."""
    terrain = routes.decode_terrain(raw)
    observed = types.SimpleNamespace(
        folder=terrain.folder, site=terrain.site, paused=True, tick=terrain.tick,
        digest='ab' * 32, horizons=(100, 100, 1000), jobs={}, buildings={}, attachments=(),
        items={item.id: types.SimpleNamespace(
            id=item.id, native_type=1, kind=item.kind.upper(), position=item.position,
            material=item.material[0], material_index=item.material[1], subtype=item.subtype,
            flags=64, stack=1, container=None, holder=None) for item in items})
    instances = []

    class ClientDouble:
        def __init__(self, authority, region, budget):
            self.closed = False
            self.captures = 0
            instances.append(self)

        def __enter__(self):
            return self

        def __exit__(self, *unused):
            self.closed = True

        def capture_once(self):
            self.captures += 1
            if self.captures != 1:
                raise AssertionError('acquisition repeated')
            self.closed = True
            return ManifestDouble(11), raw, ManifestDouble(22), b'synthetic-operations'

    def decode(operations_raw, guard):
        if operations_raw != b'synthetic-operations' or not instances[-1].closed:
            raise AssertionError('decode ran before acquisition closed')
        guard()
        return observed

    with patch.object(inventory, 'Client', ClientDouble), \
         patch.object(inventory, 'decode_operations', decode), \
         patch.object(inventory.Authority, 'guard', return_value=None):
        authority = inventory.Authority(('127.0.0.1', 5000), b'o' * 32, b'm' * 32)
        region = inventory.Region(terrain.origin, terrain.size)
        yield authority, region, observed, instances


class SequenceShellTests(unittest.TestCase):
    def setUp(self):
        self.inventory, self.cli = modules()
        self.request, self.items, self.raw = corridor()

    def test_opt_in_publishes_effective_request_and_binds_complete_decision(self):
        inv = self.inventory
        with acquired(inv, self.items, self.raw) as (authority, region, _, clients):
            budget = BudgetDouble()
            original_solver = inv.allocate_sequenced_routes
            def solver(*args, **kwargs):
                self.assertTrue(clients[-1].closed)
                self.assertEqual(budget.work_steps, 0)
                return original_solver(*args, **kwargs)
            with patch.object(inv, 'allocate_sequenced_routes', solver):
                result = json.loads(inv.run(self.request, region, authority, budget, sequence_deliveries=True))
        self.assertEqual(len(clients), 1)
        self.assertEqual(clients[0].captures, 1)
        sequence = result['result']['delivery_sequence']
        self.assertEqual(sequence['order'], ['b_far', 'a_near'])
        self.assertEqual(sequence['original_request'], self.request.json())
        self.assertEqual(result['request']['slots'][0]['after'], ['b_far'])
        self.assertEqual(result['source']['pure_allocation_digest'], result['result']['decision_digest'])
        unsigned = {key: value for key, value in result['source'].items() if key != 'evidence_digest'}
        self.assertEqual(result['source']['evidence_digest'], hashlib.sha256(
            b'dfmcp-route-inventory-evidence/1\0' + canonical(unsigned)).hexdigest())
        self.assertEqual(result['agent_turn']['briefing']['delivery_sequence'], 'reordered')
        self.assertFalse(result['runtime_admitted'])
        self.assertFalse(result['agent_turn']['briefing']['mutation_admissible'])
        self.assertEqual(inv.BINDINGS, (('operations', 'Handshake'), ('operations', 'ReadObservation'),
                                       ('map', 'Handshake'), ('map', 'ReadObservation')))

    def test_default_and_explicit_false_are_byte_identical_without_sequence_claims(self):
        inv = self.inventory
        with acquired(inv, self.items, self.raw) as (authority, region, _, clients):
            with patch.object(inv, 'allocate_sequenced_routes', side_effect=AssertionError('unexpected opt-in')):
                default = inv.run(self.request, region, authority, BudgetDouble())
                disabled = inv.run(self.request, region, authority, BudgetDouble(), sequence_deliveries=False)
        self.assertEqual(default, disabled)
        result = json.loads(default)
        self.assertEqual(result['request'], self.request.json())
        self.assertNotIn('delivery_sequence', result['result'])
        self.assertNotIn('delivery_sequence', result['agent_turn']['briefing'])
        self.assertEqual([client.captures for client in clients], [1, 1])

    def test_non_boolean_opt_in_refuses_before_native_acquisition(self):
        inv = self.inventory
        with acquired(inv, self.items, self.raw) as (authority, region, _, clients):
            for invalid in (0, 1, None, 'true', []):
                with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                    inv.run(self.request, region, authority, BudgetDouble(), sequence_deliveries=invalid)
            self.assertEqual(clients, [])

    def test_blockage_packet_has_no_plan_and_does_not_masquerade_as_shortage(self):
        inv = self.inventory
        request, items, raw = corridor(after=('a_near',))
        with acquired(inv, items, raw) as (authority, region, _, clients):
            packet = json.loads(inv.run(request, region, authority, BudgetDouble(), sequence_deliveries=True))
        self.assertTrue(packet['ok'])  # Acquisition is established by the test double only.
        self.assertEqual(packet['result']['status'], 'delivery_order_blocked')
        self.assertIsNone(packet['result']['plan'])
        self.assertIsNone(packet['result']['shortage'])
        self.assertFalse(packet['result']['global_absence_proven'])
        self.assertEqual(packet['agent_turn']['briefing']['delivery_sequence'], 'blocked')
        self.assertEqual(len(clients), 1)

    def test_source_disagreement_is_not_repaired_by_sequencing(self):
        inv = self.inventory
        for field, value in (('paused', False), ('tick', 1), ('site', 2), ('folder', 'other')):
            with self.subTest(field=field), acquired(inv, self.items, self.raw) as acquired_source:
                authority, region, observed, _ = acquired_source
                setattr(observed, field, value)
                with patch.object(inv, 'allocate_sequenced_routes', side_effect=AssertionError('invalid source accepted')):
                    with self.assertRaises(ValueError):
                        inv.run(self.request, region, authority, BudgetDouble(), sequence_deliveries=True)

    def cli_call(self, *, sequence=True, allowance=routes.MAX_WORK):
        inv, cli = self.inventory, self.cli
        output = types.SimpleNamespace(buffer=io.BytesIO())
        with acquired(inv, self.items, self.raw) as (authority, region, _, clients):
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'request.json'
                path.write_bytes(canonical(self.request.json()))
                arguments = ['--request-file', str(path), '--origin', *map(str, region.origin),
                             '--size', *map(str, region.size)]
                if sequence:
                    arguments.append('--sequence-deliveries')
                with patch.object(cli, 'Budget', lambda _: BudgetDouble(work_steps=allowance)), \
                     patch.object(cli.Authority, 'load', return_value=authority), \
                     patch.object(cli.sys, 'stdout', output):
                    status = cli.main(arguments)
        return status, output.buffer.getvalue(), clients

    def test_real_cli_reads_request_and_passes_the_explicit_option(self):
        status, raw, clients = self.cli_call()
        self.assertEqual(status, 0)
        self.assertEqual(len(raw.splitlines()), 1)
        result = json.loads(raw)
        self.assertEqual(result['result']['delivery_sequence']['order'], ['b_far', 'a_near'])
        self.assertEqual(result['request']['slots'][0]['after'], ['b_far'])
        self.assertEqual(len(clients), 1)
        self.assertTrue(clients[0].closed)

    def test_real_cli_without_option_keeps_the_original_request(self):
        status, raw, _ = self.cli_call(sequence=False)
        self.assertEqual(status, 0)
        result = json.loads(raw)
        self.assertEqual(result['request'], self.request.json())
        self.assertNotIn('delivery_sequence', result['result'])

    def test_real_cli_budget_exhaustion_publishes_one_redacted_refusal(self):
        status, raw, clients = self.cli_call(allowance=40)
        self.assertEqual(status, 2)
        self.assertEqual(len(raw.splitlines()), 1)
        result = json.loads(raw)
        self.assertFalse(result['ok'])
        self.assertIsNone(result['result'])
        self.assertIsNone(result['request'])
        self.assertIsNone(result['source'])
        self.assertEqual(result['error']['code'], 'ROUTE_FURNITURE_REFUSED')
        self.assertEqual(len(clients), 1)
        self.assertEqual(clients[0].captures, 1)
        self.assertTrue(clients[0].closed)


if __name__ == '__main__':
    unittest.main()
