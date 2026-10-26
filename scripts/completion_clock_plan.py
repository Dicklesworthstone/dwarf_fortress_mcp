"""Plan one bounded clock interval for an ORIGINAL construction stability goal.

This pure scheduler does not sample, authorize a clock, modify a goal or claim
completion. Callers must validate the original goal/journal and fresh native
source separately. All tick arithmetic is exact; deadlines remain exclusive.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; WP-10.
"""
from __future__ import annotations

from dataclasses import dataclass
import re

MAX_TICK = (2**32 - 1) * 403200 + 403199
MAX_BURST = 1200
FIELDS = ('goal_digest', 'journal_head', 'journal_frames', 'deadline', 'interval',
          'stable_samples', 'stable_span', 'max_gap', 'max_observations',
          'observations', 'streak', 'first_tick', 'counted_tick', 'last_tick',
          'last_capture')


def require(ok, message):
    if not ok:
        raise ValueError(message)


def number(value, low, high):
    require(type(value) is int and low <= value <= high, 'completion timing outside bounds')
    return value


@dataclass(frozen=True)
class Window:
    goal_digest: str
    journal_head: str
    journal_frames: int
    deadline: int
    interval: int
    stable_samples: int
    stable_span: int
    max_gap: int
    max_observations: int
    observations: int
    streak: int
    first_tick: int
    counted_tick: int
    last_tick: int
    last_capture: str

    def __post_init__(self):
        for digest in (self.goal_digest, self.journal_head, self.last_capture):
            require(type(digest) is str and re.fullmatch('[0-9a-f]{64}', digest) is not None,
                    'completion witness needs exact original identities')
        number(self.journal_frames, 1, 1030)
        number(self.deadline, 1, MAX_TICK)
        number(self.interval, 1, 403200)
        number(self.stable_samples, 2, 64)
        number(self.stable_span, 1, 4032000)
        number(self.max_gap, self.interval, 4032000)
        number(self.max_observations, self.stable_samples, 512)
        number(self.observations, 1, self.max_observations)
        number(self.streak, 1, self.observations)
        for tick in (self.first_tick, self.counted_tick, self.last_tick):
            number(tick, 0, MAX_TICK)
        require(self.first_tick <= self.counted_tick <= self.last_tick < self.deadline,
                'completion sample chronology or original deadline differs')
        require(self.counted_tick - self.first_tick >= (self.streak - 1) * self.interval,
                'completion streak cannot fit its fixed cadence')
        require(not (self.streak >= self.stable_samples
                     and self.counted_tick - self.first_tick >= self.stable_span),
                'satisfied completion history cannot request another clock')

    def json(self):
        return {name: getattr(self, name) for name in FIELDS}

    @classmethod
    def from_json(cls, value):
        require(type(value) is dict and set(value) == set(FIELDS),
                'invalid closed completion witness')
        return cls(**value)


def next_sample(window: Window, current_tick: int, *, max_ticks=MAX_BURST):
    """Choose a clock ceiling without spending the last sample too early.

    A newer paused frontier must be sampled first, rather than skipping a
    potentially qualifying observation or silently losing the original streak.
    Each burst targets an eligible new sample. No clock is offered if it cannot
    reach cadence, preserve the maximum observation gap, or fit remaining sample
    and game-time allowances. A native wall stop/overshoot is not a sample.
    """
    require(type(window) is Window, 'validated original completion witness required')
    window.__post_init__()
    number(current_tick, 0, MAX_TICK)
    number(max_ticks, 1, MAX_BURST)
    require(current_tick >= window.last_tick, 'fresh construction clock regressed')
    out = {'status': 'refused', 'reason': None, 'game_ticks': None,
           'target_tick': None, 'deadline': window.deadline,
           'observations_remaining': window.max_observations - window.observations,
           'clock_authorized': False, 'goal_policy_changed': False,
           'sampled_completion_proven': False}

    def stop(reason):
        return dict(out, reason=reason)

    if current_tick >= window.deadline:
        return stop('original_deadline_reached')
    if current_tick > window.last_tick:
        return dict(out, status='sample_required', reason='sample_newer_frontier_before_clock')
    remaining = out['observations_remaining']
    needed = max(1, window.stable_samples - window.streak)
    if remaining < needed:
        return stop('original_observation_allowance_insufficient')
    cap = min(max_ticks, window.max_gap)
    earliest = max(current_tick + 1, window.counted_tick + window.interval)
    if earliest - current_tick > cap:
        return stop('next_eligible_sample_outside_burst')
    span_end = window.first_tick + window.stable_span
    # With exactly m further eligible samples, reachable final ticks form the
    # integer interval [earliest + (m-1)*interval, current + m*cap]. Choose the
    # smallest feasible m, then the earliest first sample that can still reach
    # that interval's required span. This avoids greedy overshoot of the deadline
    # and avoids consuming the last observation before the span can be met.
    for count in range(needed, remaining + 1):
        if count > 1 and window.interval > cap:
            break
        finish = max(earliest + (count - 1) * window.interval, span_end)
        if finish >= window.deadline or finish > current_tick + count * cap:
            continue
        target = max(earliest, finish - (count - 1) * cap)
        require(current_tick < target <= current_tick + cap
                and target + (count - 1) * window.interval <= finish,
                'internal completion schedule is infeasible')
        return dict(out, status='advance_for_sample', reason='original_stability_sample_due',
                    target_tick=target, game_ticks=target - current_tick)
    return stop('stability_cannot_fit_original_window')
