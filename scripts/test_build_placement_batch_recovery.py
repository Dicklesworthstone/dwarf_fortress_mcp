"""Actual route-batch recovery handlers with injected parent, real child custody.

Only the unchanged route/planner import graph and parent batch are outside this
focused test. AST-selected production handlers are executed, not rewritten.
Full parent-custody/route-allocation integration is not claimed by these tests.
"""
import argparse
import ast
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import build_placement_client as placement
import build_placement_rpc as native
import build_placement_store as store
from build_placement_wire import canonical, integer, require
from test_build_placement_reconciliation import fixture
from test_build_placement_reconciliation_runtime import initialize, peer

SOURCE = Path(__file__).with_name('route_furniture_batch.py')


def handlers(batch_type):
    tree = ast.parse(SOURCE.read_text())
    names = {'Budget', 'Parser', 'packet', 'encoded', 'publish', 'recover', 'main'}
    selected = [n for n in tree.body if isinstance(n, (ast.ClassDef, ast.FunctionDef)) and n.name in names]
    assert {n.name for n in selected} == names
    namespace = dict(__name__='focused_route_recovery', __doc__='focused recovery',
        argparse=argparse, sys=sys, time=time, placement=placement, native=native,
        canonical=canonical, integer=integer, require=require, MAX_OUTPUT=256 * 1024,
        Batch=batch_type)
    exec(compile(ast.Module(body=selected, type_ignores=[]), str(SOURCE), 'exec'), namespace)
    return namespace


def parent_factory(plan, opened):
    class Parent:
        def __init__(self, path, identity, budget, writable=False):
            assert identity == 'b' * 64
            self.effects = store.PlacementDirectory(path, budget, writable)
            self.budget, self.writable = budget, writable
            self.plan = SimpleNamespace(steps=[SimpleNamespace(name='bed')])
            self.closed = False
            opened.append(self)
        def key(self, step):
            assert step.name == 'bed'
            return plan.key
        def register(self, step, proof):
            assert proof is None and step.name == 'bed'
            self.effects.check()
        def audit(self):
            journal = self.effects.get(plan.key)
            return {'status': 'pending_recovery' if journal.state.pending else 'all_placed',
                    'inventory_verified': True, 'batch_id': 'b' * 64,
                    'steps': [{'name': 'bed', 'phase': journal.state.terminal.phase}],
                    'retry_permitted': False}
        def check(self):
            assert not self.closed
            self.effects.check()
        def close(self):
            self.effects.close()
            self.closed = True
    return Parent


def invoke(namespace, path, operation, opened):
    class Output(io.BytesIO):
        def write(self, data):
            # This is the real parent's required publication lifetime contract.
            assert opened and not opened[-1].closed
            opened[-1].check()
            return super().write(data)
    raw = Output()
    with patch.object(sys, 'stdout', SimpleNamespace(buffer=raw)):
        status = namespace['main']([operation, '--directory', path,
            '--batch-id', 'b' * 64, '--step', 'bed'])
    assert opened[-1].closed
    return status, json.loads(raw.getvalue())


class RouteBatchRecovery(unittest.TestCase):
    def test_route_query_loads_build_only_authority_and_resolves_original_child(self):
        plan, uncertain, placed, manifest = fixture()
        opened = []
        route = handlers(parent_factory(plan, opened))
        with peer(plan, manifest, [placed]) as server, tempfile.TemporaryDirectory() as path:
            old = initialize(path, plan, uncertain, manifest, server.address)
            status, out = invoke(route, path, 'query', opened)
            self.assertEqual(status, 0)
            self.assertTrue(out['ok'])
            self.assertEqual(out['result']['status'], 'all_placed')
            self.assertEqual(out['result']['recovery']['effect_status'], 'placed')
            self.assertTrue(Path(path, store.filename(plan.key)).read_bytes().startswith(old))
            self.assertEqual(server.calls, ['Handshake', 'QueryPlacement'])
            with patch.dict(os.environ, {}, clear=True):
                status, out = invoke(route, path, 'query', opened)
            self.assertEqual(status, 0)
            self.assertFalse(out['result']['recovery']['native_contacted'])

    def test_route_cancel_retains_uncertainty_offline(self):
        plan, uncertain, _, manifest = fixture()
        opened = []
        route = handlers(parent_factory(plan, opened))
        with tempfile.TemporaryDirectory() as path, patch.dict(os.environ, {}, clear=True):
            old = initialize(path, plan, uncertain, manifest, ('127.0.0.1', 5000))
            status, out = invoke(route, path, 'cancel', opened)
            self.assertEqual(status, 0)
            self.assertTrue(out['result']['recovery']['pending'])
            self.assertFalse(out['result']['recovery']['native_contacted'])
            self.assertEqual(Path(path, store.filename(plan.key)).read_bytes(), old)

    def test_route_query_without_read_authority_never_retries_or_silently_succeeds(self):
        plan, uncertain, _, manifest = fixture()
        opened = []
        route = handlers(parent_factory(plan, opened))
        with tempfile.TemporaryDirectory() as path, patch.dict(os.environ, {}, clear=True):
            old = initialize(path, plan, uncertain, manifest, ('127.0.0.1', 5000))
            status, out = invoke(route, path, 'query', opened)
            self.assertEqual(status, 2)
            self.assertFalse(out['ok'])
            self.assertIsNone(out['result'])
            self.assertEqual(Path(path, store.filename(plan.key)).read_bytes(), old)


if __name__ == '__main__':
    unittest.main()
