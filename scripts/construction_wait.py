"""Bounded foreground execution of an existing durable construction monitor.

No worker, game-time control, new goal, or retry of a failed acquisition. Every
sample uses the original journal's read-intent/publication protocol and the SAME
shrinking operation budget. The owner and optional original-batch guard remain
held through delays and all native/publication boundaries.
Beads: df-dfhack-bridge-plane-c-pic.4 / df-dfhack-bridge-plane-c-pic.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import time
from typing import Any, Callable

MAX_SAMPLES = 32
MAX_POLL_MS = 5000
MAX_STALLED_SAMPLES = 2
# Leave cooperative time for the caller's final complete response and custody
# checks. This is a scheduling margin, not a hard real-time I/O guarantee.
FINAL_MARGIN_SECONDS = 0.25
DELAY_QUANTUM_SECONDS = 0.05
SCHEMA = 'dfmcp.construction-wait/1'
STOP_REASONS = ('terminal', 'sample_limit', 'game_tick_not_advanced',
                'rpc_allowance', 'wall_allowance')


def _require(condition: bool, detail: str) -> None:
    if not condition:
        raise ValueError(detail)


@dataclass(frozen=True)
class Limits:
    max_samples: int = 8
    poll_ms: int = 100

    def __post_init__(self) -> None:
        _require(type(self.max_samples) is int and 1 <= self.max_samples <= MAX_SAMPLES,
                 'wait requires 1..32 foreground samples')
        _require(type(self.poll_ms) is int and 10 <= self.poll_ms <= MAX_POLL_MS,
                 'wait poll delay must be 10..5000 milliseconds')


@dataclass(frozen=True)
class Result:
    limits: Limits
    samples: int
    stop_reason: str

    def view(self) -> dict:
        _require(type(self.limits) is Limits and type(self.samples) is int
                 and 0 <= self.samples <= self.limits.max_samples
                 and self.stop_reason in STOP_REASONS, 'invalid foreground wait result')
        return {'schema': SCHEMA, 'max_samples': self.limits.max_samples,
                'poll_ms': self.limits.poll_ms, 'samples_published': self.samples,
                'stop_reason': self.stop_reason, 'background_work_started': False,
                'game_time_advanced_by_wait': False, 'goal_policy_changed': False,
                'failed_acquisition_retried': False}


def reserve_view(limits: Limits) -> dict:
    """A shape/width upper bound for any final wait result under these limits."""
    return Result(limits, limits.max_samples, 'game_tick_not_advanced').view()


def add_arguments(parser: Any) -> None:
    """Defaults are applied after parsing, so other operations reject these flags."""
    parser.add_argument('--wait-samples', type=int)
    parser.add_argument('--poll-ms', type=int)


def parse_limits(args: Any) -> Limits | None:
    if args.operation != 'wait':
        _require(args.wait_samples is None and args.poll_ms is None,
                 'foreground wait options belong only to wait')
        return None
    return Limits(8 if args.wait_samples is None else args.wait_samples,
                  100 if args.poll_ms is None else args.poll_ms)


def _idle() -> None:
    """Selected-receipt monitors have no additional original-batch custody."""


def run(owner: Any, authority: Any, acquire: Callable, render: Callable,
        limits: Limits, *, source_guard: Callable[[], None] = _idle,
        sleeper: Callable[[float], None] | None = None) -> Result:
    """Drive an already opened monitor; never create, replace or close its owner.

    ``acquire(authority, original_goal, same_budget)`` is the existing one-shot
    query-only acquisition (or its fixed original-batch wrapper).
    ``render(candidate_state)`` reserves the COMPLETE response, including a
    ``reserve_view(limits)`` envelope. It runs before read intent and again before
    each sample append. Exceptions propagate without another acquisition. A
    failed acquisition leaves the owner's synchronized read intent unresolved.

    The caller retains normal final rendering, authority/custody checks, and
    error disclosure. A terminal monitor is offline and requires no authority.
    Inject ``sleeper`` in deterministic tests; it must not create background work.
    """
    _require(type(limits) is Limits, 'closed foreground wait limits required')
    limits.__post_init__()
    pause = time.sleep if sleeper is None else sleeper
    budget = owner.budget
    original_goal = owner.state.goal.digest
    original_address = owner.state.address
    samples, stalled = 0, 0

    def custody() -> None:
        budget.work()
        owner.check()
        source_guard()
        _require(owner.state.goal.digest == original_goal
                 and owner.state.address == original_address,
                 'foreground wait cannot replace its original goal or endpoint')
        budget.remaining()

    custody()
    if owner.state.progress.terminal:
        render(owner.state)
        custody()
        return Result(limits, 0, 'terminal')
    _require(authority is not None and authority.address == original_address,
             'foreground wait requires the original query endpoint')

    def guard() -> None:
        authority.guard()
        custody()

    # Four bindings + two handshakes + one page + release + both receipt
    # brackets. More pages remain charged by the existing transport itself.
    minimum_calls = 8 + 2 * len(owner.state.goal.receipts)
    _require(1 <= len(owner.state.goal.receipts) <= 32,
             'foreground wait requires a complete bounded receipt selection')
    while samples < limits.max_samples:
        guard()
        if budget.calls < minimum_calls:
            return Result(limits, samples, 'rpc_allowance')
        if budget.remaining() <= FINAL_MARGIN_SECONDS:
            return Result(limits, samples, 'wall_allowance')
        if samples:
            delay = limits.poll_ms / 1000
            if budget.remaining() <= delay + FINAL_MARGIN_SECONDS:
                return Result(limits, samples, 'wall_allowance')
            # Check revocation, original custody and cancellation between small
            # foreground delay slices, rather than hiding a long unchecked sleep.
            while delay > 0:
                guard()
                duration = min(delay, DELAY_QUANTUM_SECONDS)
                pause(duration)
                delay -= duration
                guard()

        # Output refusal or revocation must not strand a new read intent.
        render(owner.state)
        guard()
        previous_tick = owner.state.progress.last_tick
        previous_observations = owner.state.progress.observations
        owner.start_read()
        guard()
        sample = acquire(authority, owner.state.goal, budget)
        guard()

        def before_publication(candidate: Any) -> None:
            render(candidate)
            guard()

        owner.accept(sample, before_publication)
        guard()
        _require(owner.state.progress.observations == previous_observations + 1
                 and not owner.state.progress.reading,
                 'wait sample did not publish exactly one complete observation')
        samples += 1
        if owner.state.progress.terminal:
            return Result(limits, samples, 'terminal')
        tick = owner.state.progress.last_tick
        stalled = stalled + 1 if previous_tick is not None and tick == previous_tick else 0
        if stalled >= MAX_STALLED_SAMPLES:
            # This proves only repeated game ticks, not a current pause state.
            return Result(limits, samples, 'game_tick_not_advanced')
    return Result(limits, samples, 'sample_limit')
