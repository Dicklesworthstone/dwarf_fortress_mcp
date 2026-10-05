"""Deterministic foreground-owner tests; no native or live qualification.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass, replace
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import construction_wait as wait


class Budget:
    def __init__(self, seconds=60, calls=327):
        self.seconds, self.calls, self.steps = seconds, calls, 20000

    def remaining(self):
        if self.seconds <= 0:
            raise ValueError('budget expired')
        return self.seconds

    def work(self):
        self.remaining()
        if self.steps <= 0:
            raise ValueError('work exhausted')
        self.steps -= 1


@dataclass(frozen=True)
class Progress:
    observations: int = 0
    last_tick: int | None = None
    reading: bool = False
    terminal: bool = False


class Owner:
    def __init__(self, budget=None, count=1):
        self.budget = budget or Budget()
        self.goal = SimpleNamespace(digest='a' * 64, receipts=(b'receipt',) * count)
        self.state = SimpleNamespace(goal=self.goal, address=('127.0.0.1', 5000), progress=Progress())
        self.events, self.valid = [], True
        self.after_start = self.after_accept = None

    def check(self):
        if not self.valid:
            raise ValueError('custody lost')
        self.budget.remaining()

    def start_read(self):
        self.check()
        self.events.append('read_started_synced')
        self.state.progress = replace(self.state.progress, reading=True)
        if self.after_start:
            self.after_start()

    def accept(self, sample, render):
        self.check()
        tick, terminal = sample
        candidate = SimpleNamespace(goal=self.goal, address=self.state.address,
            progress=Progress(self.state.progress.observations + 1, tick, False, terminal))
        render(candidate)
        self.check()
        self.events.append('sample_synced')
        self.state = candidate
        if self.after_accept:
            self.after_accept()


class Authority:
    def __init__(self, owner):
        self.address, self.allowed = owner.state.address, True

    def guard(self):
        if not self.allowed:
            raise ValueError('authority revoked')


class WaitTests(unittest.TestCase):
    def setup_run(self, samples, owner=None):
        owner = owner or Owner()
        authority = Authority(owner)
        incoming, calls, sleeps, renders = iter(samples), [], [], []

        def acquire(a, goal, budget):
            self.assertIs(a, authority)
            self.assertIs(goal, owner.goal)
            self.assertIs(budget, owner.budget)
            self.assertTrue(owner.state.progress.reading)
            self.assertEqual(owner.events[-1], 'read_started_synced')
            owner.events.append('acquire')
            budget.calls -= 8 + 2 * len(goal.receipts)
            calls.append(True)
            value = next(incoming)
            if isinstance(value, BaseException):
                raise value
            return value

        def pause(seconds):
            self.assertGreater(seconds, 0)
            self.assertLessEqual(seconds, wait.DELAY_QUANTUM_SECONDS)
            sleeps.append(seconds)
            owner.budget.seconds -= seconds

        def render(candidate):
            renders.append(candidate.progress.observations)

        return owner, authority, acquire, pause, render, calls, sleeps, renders

    def test_completes_without_extra_sample_or_background_owner(self):
        o, a, acquire, pause, render, calls, sleeps, _ = self.setup_run([(101, False), (102, True)])
        result = wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual((result.samples, result.stop_reason), (2, 'terminal'))
        self.assertEqual(o.events, ['read_started_synced', 'acquire', 'sample_synced'] * 2)
        self.assertEqual(len(calls), 2)
        self.assertAlmostEqual(sum(sleeps), .1)
        self.assertFalse(result.view()['background_work_started'])
        self.assertFalse(result.view()['game_time_advanced_by_wait'])

    def test_sample_bound_keeps_original_goal_and_lifetime_observations(self):
        o = Owner()
        o.state.progress = Progress(observations=10, last_tick=100)
        o, a, acquire, pause, render, calls, _, _ = self.setup_run([(101, False), (102, False)], o)
        result = wait.run(o, a, acquire, render, wait.Limits(2), sleeper=pause)
        self.assertEqual(result.stop_reason, 'sample_limit')
        self.assertEqual(o.state.progress.observations, 12)
        self.assertIs(o.state.goal, o.goal)
        self.assertEqual(len(calls), 2)

    def test_terminal_is_offline_and_read_only_without_authority(self):
        o = Owner()
        o.state.progress = Progress(7, 100, False, True)
        renders = []
        def forbidden(*args):
            self.fail('terminal wait performed work')
        result = wait.run(o, None, forbidden, lambda state: renders.append(state), wait.Limits(), sleeper=forbidden)
        self.assertEqual((result.samples, result.stop_reason), (0, 'terminal'))
        self.assertEqual(o.events, [])
        self.assertEqual(len(renders), 1)

    def test_stalled_tick_returns_after_two_nonadvancing_samples(self):
        o, a, acquire, pause, render, calls, _, _ = self.setup_run([(100, False)] * 3)
        result = wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual((result.samples, result.stop_reason), (3, 'game_tick_not_advanced'))
        self.assertEqual(len(calls), 3)
        self.assertNotIn('paused', result.view())

    def test_advancement_resets_stall_count(self):
        o, a, acquire, pause, render, calls, _, _ = self.setup_run(
            [(100, False), (100, False), (101, False), (101, False), (102, True)])
        result = wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual((result.samples, result.stop_reason), (5, 'terminal'))
        self.assertEqual(len(calls), 5)

    def test_transport_failure_is_never_retried_and_keeps_read_intent(self):
        for error in (OSError('lost reply'), ValueError('bad source'), KeyboardInterrupt()):
            with self.subTest(error=type(error).__name__):
                o, a, acquire, pause, render, calls, _, _ = self.setup_run([(100, False), error, (101, True)])
                with self.assertRaises(type(error)):
                    wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
                self.assertEqual(len(calls), 2)
                self.assertEqual(o.state.progress.observations, 1)
                self.assertTrue(o.state.progress.reading)

    def test_revocation_after_read_intent_prevents_native_acquisition(self):
        o, a, acquire, pause, render, calls, _, _ = self.setup_run([(100, True)])
        o.after_start = lambda: setattr(a, 'allowed', False)
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual(calls, [])
        self.assertTrue(o.state.progress.reading)

    def test_revocation_during_delay_preserves_last_published_sample(self):
        o, a, acquire, pause, render, calls, _, _ = self.setup_run([(100, False), (101, True)])
        def revoke(seconds):
            pause(seconds)
            a.allowed = False
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, render, wait.Limits(), sleeper=revoke)
        self.assertEqual(len(calls), 1)
        self.assertFalse(o.state.progress.reading)

    def test_original_batch_loss_during_delay_prevents_next_intent(self):
        o, a, acquire, pause, render, calls, _, _ = self.setup_run([(100, False), (101, True)])
        valid = [True]
        def source_guard():
            if not valid[0]:
                raise ValueError('original batch missing')
        def lose(seconds):
            pause(seconds)
            valid[0] = False
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, render, wait.Limits(), source_guard=source_guard, sleeper=lose)
        self.assertEqual(len(calls), 1)
        self.assertFalse(o.state.progress.reading)

    def test_output_failure_before_intent_has_no_native_contact(self):
        o, a, acquire, pause, _, calls, _, _ = self.setup_run([(100, True)])
        def too_big(_):
            raise ValueError('complete response too large')
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, too_big, wait.Limits(), sleeper=pause)
        self.assertEqual(o.events, [])
        self.assertEqual(calls, [])

    def test_output_failure_after_acquisition_retains_unknown_intent(self):
        o, a, acquire, pause, _, calls, _, _ = self.setup_run([(100, True)])
        def reject_candidate(state):
            if state.progress.observations:
                raise ValueError('complete result does not fit')
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, reject_candidate, wait.Limits(), sleeper=pause)
        self.assertEqual(len(calls), 1)
        self.assertTrue(o.state.progress.reading)
        self.assertEqual(o.state.progress.observations, 0)

    def test_render_cannot_silently_revoke_authority_before_publication(self):
        o, a, acquire, pause, _, _, _, _ = self.setup_run([(100, True)])
        def revoke_candidate(state):
            if state.progress.observations:
                a.allowed = False
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, revoke_candidate, wait.Limits(), sleeper=pause)
        self.assertNotIn('sample_synced', o.events)

    def test_post_sync_custody_failure_is_not_acknowledged(self):
        o, a, acquire, pause, render, _, _, _ = self.setup_run([(100, True)])
        o.after_accept = lambda: setattr(o, 'valid', False)
        with self.assertRaises(ValueError):
            wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual(o.state.progress.observations, 1)

    def test_rpc_allowance_is_not_renewed_for_32_targets(self):
        o, a, acquire, pause, render, calls, _, _ = self.setup_run([(i, False) for i in range(5)], Owner(count=32))
        result = wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual((result.samples, result.stop_reason), (4, 'rpc_allowance'))
        self.assertEqual(len(calls), 4)
        self.assertEqual(o.budget.calls, 39)
        self.assertFalse(o.state.progress.reading)

    def test_wall_margin_stops_before_an_unfinishable_delay(self):
        o, a, acquire, pause, render, calls, sleeps, _ = self.setup_run([(100, False)], Owner(Budget(.3)))
        result = wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
        self.assertEqual((result.samples, result.stop_reason), (1, 'wall_allowance'))
        self.assertEqual(len(calls), 1)
        self.assertEqual(sleeps, [])

    def test_insufficient_initial_allowance_does_not_create_intent(self):
        for budget, reason in ((Budget(.1), 'wall_allowance'), (Budget(calls=9), 'rpc_allowance')):
            with self.subTest(reason=reason):
                o, a, acquire, pause, render, calls, _, _ = self.setup_run([], Owner(budget))
                result = wait.run(o, a, acquire, render, wait.Limits(), sleeper=pause)
                self.assertEqual((result.samples, result.stop_reason), (0, reason))
                self.assertEqual(calls, [])
                self.assertEqual(o.events, [])

    def test_owner_goal_and_endpoint_cannot_change_during_wait(self):
        for field, value in (('goal', SimpleNamespace(digest='b' * 64)), ('address', ('127.0.0.1', 6000))):
            with self.subTest(field=field):
                o, a, acquire, pause, render, calls, _, _ = self.setup_run([(100, False), (101, True)])
                def substitute(seconds):
                    pause(seconds)
                    setattr(o.state, field, value)
                with self.assertRaises(ValueError):
                    wait.run(o, a, acquire, render, wait.Limits(), sleeper=substitute)
                self.assertEqual(len(calls), 1)

    def test_closed_limits_reject_booleans_floats_and_unbounded_values(self):
        for value in (True, False, 0, -1, 33, 1.0, None, '2'):
            with self.subTest(samples=value), self.assertRaises(ValueError):
                wait.Limits(value)
        for value in (True, 0, 9, 5001, 10.0, None, '10'):
            with self.subTest(poll=value), self.assertRaises(ValueError):
                wait.Limits(poll_ms=value)

    def test_wait_flags_are_refused_for_every_other_operation(self):
        parser = argparse.ArgumentParser()
        parser.add_argument('operation')
        wait.add_arguments(parser)
        self.assertEqual(wait.parse_limits(parser.parse_args(['wait'])), wait.Limits())
        for operation in ('start', 'sample', 'inspect', 'cancel'):
            self.assertIsNone(wait.parse_limits(parser.parse_args([operation])))
            for option in ('--wait-samples', '--poll-ms'):
                with self.subTest(operation=operation, option=option), self.assertRaises(ValueError):
                    wait.parse_limits(parser.parse_args([operation, option, '10']))

    def test_reserved_envelope_bounds_all_final_results(self):
        import json
        for limits in (wait.Limits(), wait.Limits(32, 5000), wait.Limits(1, 10)):
            bound = len(json.dumps(wait.reserve_view(limits), sort_keys=True))
            for count in range(limits.max_samples + 1):
                for reason in wait.STOP_REASONS:
                    value = wait.Result(limits, count, reason).view()
                    self.assertLessEqual(len(json.dumps(value, sort_keys=True)), bound)


if __name__ == '__main__':
    unittest.main()
