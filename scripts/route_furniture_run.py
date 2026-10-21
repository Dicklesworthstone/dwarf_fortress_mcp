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
    _room_scope(batch, state)
    count = integer(state['placed'], 0, len(batch.plan.steps) - 1)
    approved = tuple(step.name for step in batch.plan.ordered[count:count + limits.max_steps])
    require(approved and state['next_step'] == approved[0], 'run prefix differs from original canonical order')
    return approved



# Local journal eligibility and observed execution readiness are different.
# A pending construction does not change original custody, but must stop effects.
UNREADY_FRONTIERS = frozenset((
    'route_blocked', 'route_unestablished', 'construction_pending',
    'construction_unestablished', 'construction_blocked',
    'room_blocked', 'room_unestablished',
))
ROOM_FRONTIERS = frozenset(('room_blocked', 'room_unestablished'))
ROOM_CUSTODY_FIELDS = ('room_handoff_digest', 'room_plan_digest',
                       'original_room_plan', 'complete_original_room_intent_retained')
CUSTODY_FIELDS = ('batch_id', 'review_digest', 'inventory_digest', 'plan_digest',
                  'placed', 'next_step', 'pending_step', 'status', 'stopped')


def _room_scope(batch, state):
    """Carry the authenticated parent's identity without inventing room custody.

    Batch.audit owns decoding and file checks. This boundary only preserves its
    complete parent through the multi-step owner's copies and confirmation.
    Legacy furniture-only scopes retain their original canonical bytes.
    """
    parent = getattr(batch, 'room_handoff', None)
    if parent is None:
        require(not any(name in state for name in ROOM_CUSTODY_FIELDS),
                'furniture-only run cannot adopt a room parent')
        return {}
    require(state.get('room_handoff_digest') == parent.digest
            and state.get('complete_original_room_intent_retained') is True
            and type(state.get('original_room_plan')) is dict,
            'run lost the complete original room parent')
    digest_text(state.get('room_handoff_digest'))
    digest_text(state.get('room_plan_digest'))
    require(state['original_room_plan'].get('plan_digest') == state['room_plan_digest'],
            'run room plan differs from original parent identity')
    return {name: state[name] for name in ('room_handoff_digest', 'room_plan_digest')}


def _room_review(batch, before, observed, after):
    """A well-formed unready room is an observation, never a transport failure."""
    scope = _room_scope(batch, before)
    require(_room_scope(batch, observed) == _room_scope(batch, after) == scope,
            'room parent changed during run review')
    report = observed['route_frontier']
    if not scope:
        require(report['status'] not in ROOM_FRONTIERS,
                'room readiness reported without an original room')
        return
    require(all(before[key] == observed[key] == after[key] for key in ROOM_CUSTODY_FIELDS),
            'run readiness omitted or changed original room intent')
    require(observed.get('room_constraints_checked_this_call') is True
            and report.get('room_constraints_checked_this_call') is True
            and all(report.get(key) == value for key, value in scope.items()),
            'room-backed run requires a current original-room review')
    terrain = report.get('room_terrain')
    require(type(terrain) is dict and terrain.get('status') in ('matched', 'contradicted', 'unknown'),
            'room review omitted its terrain diagnosis')
    if report['status'] in ROOM_FRONTIERS | {'ready'}:
        expected = {'ready': 'matched', 'room_blocked': 'contradicted',
                    'room_unestablished': 'unknown'}[report['status']]
        require(report.get('execution_frontier_status') == 'ready'
                and terrain['status'] == expected
                and terrain.get('all_required_shapes_at_sample') is (expected == 'matched'),
                'room readiness disagrees with its complete diagnosis')
    else:
        require(report.get('execution_frontier_status') == report['status'],
                'room review replaced an earlier execution blocker')


def _review_status(batch, before, observed):
    """Bind a complete read to unchanged original custody, not to permission.

    The one-step owner may narrow advance_allowed after a successful acquisition.
    That is a valid unready observation, not damaged custody or native failure.
    Unknown statuses and contradictory confirmation fields still fail closed.
    """
    after = batch.audit()
    require(before['advance_allowed'] is True and after['advance_allowed'] is True
            and before['status'] == 'ready' and not before['stopped'],
            'original custody cannot admit a readiness review')
    for state in (before, observed, after):
        require(state['inventory_verified'] is True and state['batch_id'] == batch.id
                and state['review_digest'] == batch.review.digest,
                'readiness review differs from original custody')
    require(all(before[key] == observed[key] == after[key] for key in CUSTODY_FIELDS),
            'original inventory changed during readiness review')
    _room_review(batch, before, observed, after)
    status = observed['route_frontier']['status']
    require(type(status) is str and status in UNREADY_FRONTIERS | {'ready'},
            'unknown execution readiness status')
    if status == 'ready':
        require(observed['advance_allowed'] is True, 'ready review withheld advancement')
        digest_text(observed['expected_plan'])
        digest_text(observed['confirm_review'])
    else:
        require(observed['expected_plan'] is None and observed['confirm_review'] is None,
                'unready observation cannot issue placement confirmation')
        observed['advance_allowed'] = False
    return status


def _confirmation(batch, state, limits, approved):
    require(state.get('confirm_review') is not None and state.get('expected_plan') is not None,
            'run requires a ready first-step review')
    digest_text(state['inventory_digest'])
    digest_text(state['confirm_review'])
    digest_text(state['expected_plan'])
    data = {'schema': SCHEMA, 'policy': POLICY, 'batch_id': batch.id,
            'review_digest': batch.review.digest, 'inventory_digest': state['inventory_digest'],
            'first_native_plan': state['expected_plan'], 'first_step_confirmation': state['confirm_review'],
            'approved_steps': list(approved), 'limits': limits.json(),
            **_room_scope(batch, state)}
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
    before = batch.audit()
    approved = _scope(batch, before, limits)
    state = steps.review_step(batch, authority)
    status = _review_status(batch, before, state)
    scope, confirmation = (None, None)
    if status == 'ready':
        scope, confirmation = _confirmation(batch, state, limits, approved)
    state['run_review'] = {'policy': POLICY, 'scope': scope, 'confirm_run': confirmation,
                           'automatic_retry_permitted': False, 'max_steps': limits.max_steps,
                           'construction_completion_proven': False}
    _reserve(steps, state)
    steps.encoded('review-run', state)
    authority.guard()
    batch.check()
    return state


def _finish(steps, batch, authority, scope, outcomes, stop_reason, *, faulted=False,
            observation=None):
    # Audit reads local original journals only. On lost custody/deadline this may
    # itself refuse; callers then keep the on-disk history and emit the usual
    # sanitized error, rather than converting cached progress into current proof.
    state = batch.audit()
    require(scope is None or (scope['batch_id'] == batch.id
            and scope['review_digest'] == batch.review.digest), 'run origin changed')
    room_scope = _room_scope(batch, state)
    if scope is not None:
        require({key: scope[key] for key in room_scope if key in scope} == room_scope
                and (bool(room_scope) or not any(key in scope for key in ROOM_CUSTODY_FIELDS)),
                'run confirmation lost its original room scope')
    if observation is not None:
        status = _review_status(batch, state, observation)
        require(status == stop_reason and status in UNREADY_FRONTIERS,
                'stopped run observation disagrees with its reason')
        state['route_frontier'] = observation['route_frontier']
    # Custody's ready means no unresolved child, not currently safe to execute.
    # Never manufacture a fresh permit from the final offline journal audit.
    state.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                 route_constraints_checked_this_call=observation is not None,
                 execution_review_required=True)
    if room_scope:
        state['room_constraints_checked_this_call'] = observation is not None
    state['run'] = {'schema': SCHEMA, 'policy': POLICY, 'scope': scope,
                    'stop_reason': stop_reason, 'outcomes': outcomes,
                    'confirmed_placements_this_call': sum(row['phase'] == 'placed' for row in outcomes),
                    'advance_invocations_this_call': len(outcomes), 'faulted': faulted,
                    'confirmation_validated': scope is not None,
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
    approved = _scope(batch, initial, limits)  # Refuse unresolved work BEFORE native I/O.
    require(_admit_step(batch), 'first run step does not fit aggregate allowance')
    first = steps.review_step(batch, authority)
    status = _review_status(batch, initial, first)
    if status != 'ready':
        # Well-formed consent is not validated by an unready observation. Return
        # a read-only stop with no approved scope and no attempted placement.
        return _finish(steps, batch, authority, None, [], status, observation=first)
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
            try:
                current = steps.review_step(batch, authority)
                status = _review_status(batch, state, current)
            except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
                # A failed read is not a placement attempt. Retain earlier
                # outcomes if custody still permits, but never an older review.
                return _finish(steps, batch, authority, scope, outcomes, 'review_failed', faulted=True)
            if status != 'ready':
                return _finish(steps, batch, authority, scope, outcomes, status, observation=current)
        require(current['next_step'] == name and current['advance_allowed'], 'run frontier changed')
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
