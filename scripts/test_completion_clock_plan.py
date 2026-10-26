"""Actual scheduler versus exhaustive advancing-sample schedules, no native I/O."""
from dataclasses import replace
from functools import lru_cache
import itertools
import random
import unittest

from completion_clock_plan import Window, MAX_TICK, next_sample


def window(**changes):
    base = Window('a' * 64, 'b' * 64, 3, 100, 2, 2, 5, 10, 8,
                  1, 1, 10, 10, 10, 'c' * 64)
    return replace(base, **changes)


def exhaustive(w, cap):
    cap = min(cap, w.max_gap)
    @lru_cache(None)
    def visit(tick, counted, count, remaining):
        if count >= w.stable_samples and counted - w.first_tick >= w.stable_span:
            return True
        if not remaining:
            return False
        return any(visit(t, t, count + 1, remaining - 1) for t in range(
            max(tick + 1, counted + w.interval), min(tick + cap, w.deadline - 1) + 1))
    return visit(w.last_tick, w.counted_tick, w.streak, w.max_observations - w.observations), visit


class CadenceTests(unittest.TestCase):
    def test_last_needed_sample_waits_for_span_without_renewing_policy(self):
        w = window()
        before = w.json()
        out = next_sample(w, 10)
        self.assertEqual((out['status'], out['game_ticks'], out['target_tick']),
                         ('advance_for_sample', 5, 15))
        self.assertEqual(w.json(), before)
        self.assertFalse(out['clock_authorized'])
        self.assertFalse(out['sampled_completion_proven'])

    def test_additional_counts_keep_the_original_interval(self):
        w = window(stable_samples=4)
        self.assertEqual(next_sample(w, 10)['game_ticks'], 2)

    def test_early_poll_uses_last_counted_tick_not_an_invented_cadence(self):
        w = window(interval=5, stable_span=1, last_tick=13, max_gap=8)
        self.assertEqual(next_sample(w, 13)['target_tick'], 15)

    def test_small_bursts_preserve_long_span_without_spending_last_sample_early(self):
        w = window(stable_span=9, max_observations=4)
        self.assertEqual(next_sample(w, 10, max_ticks=3)['target_tick'], 13)
        w = replace(w, observations=2, streak=2, counted_tick=13, last_tick=13)
        self.assertEqual(next_sample(w, 13, max_ticks=3)['target_tick'], 16)
        w = replace(w, observations=3, streak=3, counted_tick=16, last_tick=16)
        self.assertEqual(next_sample(w, 16, max_ticks=3)['target_tick'], 19)

    def test_cannot_use_more_samples_or_cross_exclusive_deadline(self):
        for w in (window(max_observations=2, stable_span=11),
                  window(deadline=15), window(max_observations=4, observations=3,
                                             stable_samples=4)):
            self.assertEqual(next_sample(w, w.last_tick)['status'], 'refused')

    def test_newer_frontier_must_be_sampled_first_and_regression_refuses(self):
        self.assertEqual(next_sample(window(), 12)['status'], 'sample_required')
        self.assertEqual(next_sample(window(), 100)['reason'], 'original_deadline_reached')
        with self.assertRaises(ValueError): next_sample(window(), 9)

    def test_satisfied_unknown_interrupted_and_malformed_windows_are_not_eligible(self):
        for change in ({'last_tick': None}, {'streak': 0}, {'observations': 0},
                       {'stable_samples': True}, {'max_gap': 1}, {'journal_head': 'BAD'},
                       {'streak': 3, 'observations': 3, 'counted_tick': 11, 'last_tick': 11},
                       {'streak': 2, 'observations': 2, 'counted_tick': 15, 'last_tick': 15}):
            with self.subTest(change=change), self.assertRaises(ValueError): window(**change)
        for cap in (0, True, 1201, 1.5):
            with self.assertRaises(ValueError): next_sample(window(), 10, max_ticks=cap)
        with self.assertRaises(ValueError): Window.from_json(dict(window().json(), reading=True))

    def test_large_tick_arithmetic_is_exact(self):
        w = window(first_tick=MAX_TICK - 20, counted_tick=MAX_TICK - 20,
                   last_tick=MAX_TICK - 20, deadline=MAX_TICK)
        self.assertEqual(next_sample(w, w.last_tick)['target_tick'], MAX_TICK - 15)

    def test_exhaustive_small_schedules_and_selected_first_step_are_sound(self):
        checked = 0
        for interval, cap, samples, span, left, deadline in itertools.product(
                range(1, 4), range(1, 5), range(2, 5), range(1, 11), range(1, 5), (4, 8, 13)):
            w = window(first_tick=0, counted_tick=0, last_tick=0, deadline=deadline,
                       interval=interval, max_gap=max(interval, cap), stable_samples=samples,
                       stable_span=span, observations=1,
                       max_observations=max(samples, left + 1))
            # Use exactly 'left' residual observations, including histories whose
            # earlier observations did not add to the current streak.
            w = replace(w, observations=w.max_observations - left)
            exists, visit = exhaustive(w, cap)
            out = next_sample(w, 0, max_ticks=cap)
            self.assertEqual(out['status'] == 'advance_for_sample', exists, (w, cap, out))
            if exists:
                t = out['target_tick']
                self.assertTrue(visit(t, t, w.streak + 1, left - 1), (w, cap, out))
            checked += 1
        self.assertEqual(checked, 4320)

    def test_seeded_existing_streaks_and_early_polls_match_independent_oracle(self):
        rng = random.Random(7193)
        checked = 0
        for _ in range(1200):
            interval = rng.randint(1, 4)
            count = rng.randint(1, 3)
            counted = (count - 1) * interval + rng.randint(0, 2)
            if count == 1: counted = 0
            last = counted + rng.randrange(interval)
            samples, span = rng.randint(2, 4), rng.randint(1, 14)
            if count >= samples and counted >= span: continue
            total = max(samples, count + rng.randint(1, 4))
            w = window(first_tick=0, counted_tick=counted, last_tick=last,
                       deadline=last + rng.randint(1, 16), interval=interval,
                       max_gap=rng.randint(interval, 6), stable_samples=samples,
                       stable_span=span, observations=count, streak=count,
                       max_observations=total)
            cap = rng.randint(1, 6)
            exists, visit = exhaustive(w, cap)
            out = next_sample(w, last, max_ticks=cap)
            self.assertEqual(out['status'] == 'advance_for_sample', exists, (w, cap, out))
            if exists:
                t = out['target_tick']
                self.assertTrue(visit(t, t, count + 1, total - count - 1), (w, cap, out))
            checked += 1
        self.assertGreater(checked, 800)


if __name__ == '__main__':
    unittest.main()
