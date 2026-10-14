"""Explicitly confirmed, bounded foreground execution of an original route batch.

This owner composes the existing one-step review/advance path. A run confirmation
covers a fixed original prefix and resource limits; it is not a new native permit.
Each step still creates its own synchronized intent and obtains a fresh native
preparation. Unknown outcomes stop, never retry, query, cancel, skip or substitute.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib

from furniture_plan import MAX_STEPS, canonical, integer, require
from route_furniture_review import digest_text

POLICY = 'explicit-original-prefix-with-fresh-per-step-gates/1'
SCHEMA = 'dfmcp.route-furniture-run/1'
# Includes one review and the existing two-bracket advance. These are admission
# thresholds, not refunds: individual operations consume the same budget object.
STEP_CALLS = 840
STEP_NETWORK_BYTES = 60 * 1024 * 1024
STEP_WORK = 26_000_000
FINAL_WORK = 100_000
OUTCOME_RESERVATION = 24 * 1024


@dataclass(frozen=True)
class Limits:
    max_steps: int
    timeout_ms: int = 10000

    def __post_init__(self):
        integer(self.max_steps, 1, MAX_STEPS)
        integer(self.timeout_ms, 1, 60000)

    def json(self):
        return {'max_steps': self.max_steps, 'timeout_ms': self.timeout_ms,
                'maximum_calls': STEP_CALLS * self.max_steps,
                'maximum_network_bytes': STEP_NETWORK_BYTES * self.max_steps,
                'maximum_work_units': STEP_WORK * (self.max_steps + 1) + FINAL_WORK,
                'maximum_disk_bytes': 1024 * 1024 * 1024}


def _step_api(provided):
    # The script entrypoint supplies its own module to avoid importing a second
    # copy of its Authority class under __main__. This is an embedding port,
    # never a command-line selector or native-method override.
    if provided is None:
        import route_furniture_batch as provided
    return provided


def make_budget(limits: Limits, *, _steps=None):
    """Allocate ONCE, before opening custody; no per-step deadline or quota reset."""
    require(type(limits) is Limits, 'closed run limits required')
    limits.__post_init__()
    steps = _step_api(_steps)
    budget = steps.Budget(limits.timeout_ms)
    maximum = limits.json()
    budget.calls = maximum['maximum_calls']
    budget.network_bytes = maximum['maximum_network_bytes']
    budget.work_steps = maximum['maximum_work_units']
    budget.run_limits = limits
    return budget


def _limits(batch, limits):
    require(type(limits) is Limits and getattr(batch.budget, 'run_limits', None) == limits,
            'run requires its original aggregate budget and limits')
    limits.__post_init__()
    maximum = limits.json()
    for field, name in (('calls', 'maximum_calls'), ('network_bytes', 'maximum_network_bytes'),
                        ('work_steps', 'maximum_work_units'), ('disk_bytes', 'maximum_disk_bytes')):
        value = getattr(batch.budget, field)
        require(type(value) is int and 0 <= value <= maximum[name], 'run allowance increased or malformed')
    batch.budget.remaining()


def _admit_step(batch):
    budget = batch.budget
    budget.remaining()
    return (budget.calls >= STEP_CALLS and budget.network_bytes >= STEP_NETWORK_BYTES
            and budget.work_steps >= STEP_WORK + FINAL_WORK and budget.disk_bytes >= 1024 * 1024)


def _scope(batch, state, limits):
    require(state['inventory_verified'] and state['batch_id'] == batch.id
            and state['review_digest'] == batch.review.digest
            and state['advance_allowed'], 'original batch is not ready for run review')
    count = integer(state['placed'], 0, len(batch.plan.steps) - 1)
    approved = tuple(step.name for step in batch.plan.ordered[count:count + limits.max_steps])
    require(approved and state['next_step'] == approved[0], 'run prefix differs from original canonical order')
    return approved


def _confirmation(batch, state, limits, approved):
    require(state.get('confirm_review') is not None and state.get('expected_plan') is not None,
            'run requires a ready first-step review')
    digest_text(state['inventory_digest'])
    digest_text(state['confirm_review'])
    digest_text(state['expected_plan'])
    data = {'schema': SCHEMA, 'policy': POLICY, 'batch_id': batch.id,
            'review_digest': batch.review.digest, 'inventory_digest': state['inventory_digest'],
            'first_native_plan': state['expected_plan'], 'first_step_confirmation': state['confirm_review'],
            'approved_steps': list(approved), 'limits': limits.json()}
    return data, hashlib.sha256(b'dfmcp-route-run-confirmation/1\0' + canonical(data)).hexdigest()


def _reserve(steps, state):
    # The complete original state and all possible compact run outcomes must fit
    # before the next child can be created. The native coordinator independently
    # reserves its full one-step result; we do not keep 32 copies of that result.
    require(len(steps.encoded('run', state)) + OUTCOME_RESERVATION + 96 * 1024 <= steps.MAX_OUTPUT,
            'complete original batch and run outcomes cannot fit')


def review_run(batch, authority, limits: Limits, *, _steps=None):
    """Read-only proposal for one exact bounded prefix, never automatic consent."""
    steps = _step_api(_steps)
    _limits(batch, limits)
    authority.guard()
    state = steps.review_step(batch, authority)
    approved = _scope(batch, state, limits)
    scope, confirmation = (None, None)
    if state['confirm_review'] is not None:
        scope, confirmation = _confirmation(batch, state, limits, approved)
    state['run_review'] = {'policy': POLICY, 'scope': scope, 'confirm_run': confirmation,
                           'automatic_retry_permitted': False, 'max_steps': limits.max_steps,
                           'construction_completion_proven': False}
    _reserve(steps, state)
    steps.encoded('review-run', state)
    authority.guard()
    batch.check()
    return state


def _finish(steps, batch, authority, scope, outcomes, stop_reason, *, faulted=False):
    # Audit reads local original journals only. On lost custody/deadline this may
    # itself refuse; callers then keep the on-disk history and emit the usual
    # sanitized error, rather than converting cached progress into current proof.
    state = batch.audit()
    state['run'] = {'schema': SCHEMA, 'policy': POLICY, 'scope': scope,
                    'stop_reason': stop_reason, 'outcomes': outcomes,
                    'confirmed_placements_this_call': sum(row['phase'] == 'placed' for row in outcomes),
                    'advance_invocations_this_call': len(outcomes), 'faulted': faulted,
                    'automatic_recovery_attempted': False, 'automatic_retry_permitted': False,
                    'clock_control_performed': False, 'construction_completion_proven': False}
    steps.encoded('run', state, error=faulted)
    authority.guard()
    batch.check()
    return state


def _stop(state):
    if state['status'] == 'all_placed':
        return 'all_placed'
    if state['stopped']:
        return 'batch_stopped'
    if not state['advance_allowed']:
        return state['status'] if state['status'] != 'ready' else 'original_custody_incomplete'
    return None


def run(batch, authority, limits: Limits, confirmed: str, *, _steps=None):
    """Execute at most the explicitly confirmed original prefix on this thread.

    A fresh run review must match the external confirmation before the first
    effect. Subsequent exact original steps get fresh paired per-step reviews
    under that bounded consent; every existing final-route, source, permission,
    journal and one-use-native-permit gate still runs. No failed step is retried.
    After any progress, replaying the old run confirmation fails on inventory.
    """
    steps = _step_api(_steps)
    digest_text(confirmed)
    _limits(batch, limits)
    require(batch.writable, 'batch run requires original writable custody')
    authority.guard('CommitPlacement')
    initial = batch.audit()
    _scope(batch, initial, limits)  # Refuse stopped/pending work BEFORE native I/O.
    require(_admit_step(batch), 'first run step does not fit aggregate allowance')
    first = steps.review_step(batch, authority)
    approved = _scope(batch, first, limits)
    scope, actual = _confirmation(batch, first, limits, approved)
    require(actual == confirmed, 'run confirmation differs from current original prefix or limits')
    outcomes = []
    current = first
    for index, name in enumerate(approved):
        _limits(batch, limits)
        authority.guard('CommitPlacement')
        if index:
            state = batch.audit()
            reason = _stop(state)
            if reason is not None:
                return _finish(steps, batch, authority, scope, outcomes, reason)
            require(state['next_step'] == name, 'run cannot skip or reorder original work')
            if not _admit_step(batch):
                return _finish(steps, batch, authority, scope, outcomes, 'aggregate_budget_boundary')
            current = steps.review_step(batch, authority)
        require(current['next_step'] == name and current['advance_allowed'], 'run frontier changed')
        if current['confirm_review'] is None or current['expected_plan'] is None:
            return _finish(steps, batch, authority, scope, outcomes, current['route_frontier']['status'])
        _reserve(steps, current)
        # Record an invocation, not a fabricated claim of native dispatch. If the
        # coordinator raises after dispatch sync, query the local journal only.
        outcome = {'slot': name, 'native_plan_digest': current['expected_plan'],
                   'phase': 'unknown', 'receipt_digest': None, 'dispatch_recorded': None}
        outcomes.append(outcome)
        try:
            completed = steps.advance(batch, current['expected_plan'], current['confirm_review'], authority)
        except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
            state = batch.audit()
            row = next(row for row in state['steps'] if row['name'] == name)
            outcome.update(phase=row['phase'], receipt_digest=row.get('receipt_digest'),
                           dispatch_recorded=row.get('dispatch_recorded', False))
            return _finish(steps, batch, authority, scope, outcomes, 'step_failed', faulted=True)
        require(completed['advanced_step'] == name, 'coordinator advanced a different original step')
        state = batch.audit()
        row = next(row for row in state['steps'] if row['name'] == name)
        outcome.update(phase=row['phase'], receipt_digest=row.get('receipt_digest'),
                       dispatch_recorded=row.get('dispatch_recorded', False))
        reason = _stop(state)
        if row['phase'] != 'placed' or reason is not None:
            return _finish(steps, batch, authority, scope, outcomes, reason or 'original_step_unresolved')
    return _finish(steps, batch, authority, scope, outcomes, 'step_limit')
