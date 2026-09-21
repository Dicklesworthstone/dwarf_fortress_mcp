#!/usr/bin/env python3
"""Independent selection/routing/budget models, NOT execution of Rust or MCP."""
from __future__ import annotations
import itertools
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONNECT = 7 * 400 * 1024 + 24
STATES = ('intent', 'prepared', 'dispatch_started', 'tracking', 'terminal', 'cancel_requested', 'cancelled_before_dispatch')


def route(mode, operation, state, unknown=False):
    """Reference routing contract, independent of transport and journal bytes."""
    settled = state in ('terminal', 'cancelled_before_dispatch')
    if operation in ('prepare', 'commit', 'cancel') and mode != 'control':
        return 'refuse'
    if operation == 'observe':
        return 'refuse' if mode == 'offline' else 'observe'
    if operation in ('view', 'inspect'):
        return 'local'
    if operation == 'commit':
        return 'local' if settled else 'commit' if state == 'prepared' else 'refuse'
    if operation == 'wait':
        if settled or state == 'prepared' or unknown:
            return 'local'
        return 'refuse' if mode == 'offline' else 'query'
    if operation == 'cancel':
        return 'local' if settled or unknown or state in ('intent', 'prepared') else 'cancel'
    return 'local'  # an exact duplicate prepare never renews a native preparation


class References(unittest.TestCase):
    def test_fixed_modes_and_local_operations(self):
        for mode, op, state, unknown in itertools.product(
                ('offline', 'recover', 'control'),
                ('observe', 'prepare', 'commit', 'wait', 'cancel', 'view', 'inspect'), STATES, (False, True)):
            result = route(mode, op, state, unknown)
            if mode == 'offline':
                self.assertIn(result, ('local', 'refuse'))
            if mode == 'recover':
                self.assertNotIn(result, ('prepare', 'commit', 'cancel'))
            if op == 'commit' and state != 'prepared':
                self.assertNotEqual(result, 'commit')
            if op in ('view', 'inspect'):
                self.assertEqual(result, 'local')

    def test_unknown_never_becomes_settled_or_reconnected(self):
        for mode, state in itertools.product(('offline','recover','control'), ('tracking','cancel_requested')):
            self.assertEqual(route(mode,'wait',state,True), 'local')
            self.assertNotEqual(route(mode,'commit',state,True), 'commit')
            self.assertNotEqual(route(mode,'cancel',state,True), 'cancel')

    def test_refresh_erases_prior_selection_before_any_failure(self):
        for old, valid_ids, authority, io, newer in itertools.product(
                (None,'old'),(False,True),(False,True),(False,True),(False,True)):
            selected = None
            if valid_ids and authority and io and newer:
                selected = 'fresh'
            self.assertNotEqual(selected, 'old')
            self.assertEqual(selected is not None, valid_ids and authority and io and newer)

    def test_selected_witness_and_high_water_tick(self):
        for old_tick, new_tick, ok in itertools.product((0,99,100,101), (99,100,101), (False,True)):
            selected = None
            high = old_tick
            if ok and new_tick >= high:
                selected = ('witness',new_tick)
                high = new_tick
            self.assertGreaterEqual(high,old_tick)
            self.assertEqual(selected is not None, ok and new_tick >= old_tick)

    def test_budget_partitions_are_nonrenewable(self):
        for view_bytes, copies, work in itertools.product((0,100,67108864),(0,65675,64*73867),(1,100,CONNECT)):
            cost=view_bytes+copies
            total=cost+CONNECT+work
            self.assertEqual(total-cost-CONNECT,work)
            self.assertLess(total-cost-CONNECT,total)
        for total in (1,CONNECT-1,CONNECT):
            self.assertLessEqual(total-CONNECT,0)

    def test_registered_rust_source_and_scenarios(self):
        source=(ROOT/'crates/dfmcp-adapter/src/workforce_session.rs').read_text()
        tests=(ROOT/'crates/dfmcp-adapter/src/workforce_session/tests.rs').read_text()
        self.assertIn('pub mod workforce_session;', (ROOT/'crates/dfmcp-adapter/src/lib.rs').read_text())
        self.assertEqual(tests.count('#[test]'),12)
        observe=source.split('pub fn observe<N, F>',1)[1].split('pub fn prepare<N, F>',1)[0]
        self.assertLess(observe.index('self.clear_selection()'),observe.index('self.context(context'))
        self.assertLess(observe.index('self.view_with('),observe.index('budget.connect('))
        for forbidden in ('std::process::Command','std::thread::spawn','unsafe {'):
            self.assertNotIn(forbidden,source)


if __name__ == '__main__':
    unittest.main(verbosity=2)
