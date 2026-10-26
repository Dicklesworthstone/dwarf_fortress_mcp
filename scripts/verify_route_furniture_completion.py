#!/usr/bin/env python3
"""Advance to one original construction stability sample, never certify it by time.

Review and advance hold the complete original batch and completion journal
read-only. An independently reviewed confirmation binds both journals, the
original timing policy and an exact fresh quiescent native frontier. Only the
existing completion sampler may subsequently append evidence or satisfy a goal.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; WP-10.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from pathlib import Path
import sys

import bounded_run_client as clock
import bounded_run_outcomes as outcomes
from completion_clock_plan import Window, next_sample
from completion_clock_scope import CompletionScope, POLICY, SCHEMA
from furniture_plan import canonical, integer, require
import route_furniture_batch as route
from route_furniture_batch_store import Batch
from route_furniture_clock_context import Client, ScopeStore
from route_furniture_clock_inventory import Inventory
from route_furniture_completion import verify_origin
from route_furniture_completion_store import Journal
import route_furniture_final as final
from route_furniture_progress import ProgressAuthority
from route_furniture_review import digest_text

MAX_OUTPUT = 256 * 1024
MAX_CALLS = 2 * (13 + 2 * 32 + 255) + 8 + 10
MAX_NETWORK = 48 * 1024 * 1024
CLOCK_DISK = 256 * 1024


def make_budget(timeout_ms):
    budget = route.Budget(timeout_ms)
    # One allocation before opening either original owner. No per-read refill.
    budget.calls, budget.network_bytes = MAX_CALLS, MAX_NETWORK
    return budget


def witness(owner, batch, expected_goal):
    digest_text(expected_goal)
    require(type(owner) is Journal and not owner.writable and not batch.writable
            and owner.budget is batch.budget and owner.state.address == batch.address,
            'verification requires read-only original owners and one budget')
    require(owner.state.goal.digest == expected_goal, 'original completion goal differs')
    verify_origin(owner.state.goal, batch)
    owner.check()
    state, goal = owner.state.progress, owner.state.goal
    require(state.goal_digest == expected_goal and not state.terminal and not state.reading
            and state.phase == 'candidate' and state.streak > 0,
            'sample the original incomplete construction goal before clock verification')
    require(len(state.assessments) == len(goal.receipts)
            and all(row.condition.status == 'condition_met' for row in state.assessments),
            'complete original conditions must hold in the retained sample')
    values = {name: getattr(goal.condition, name) for name in (
        'deadline', 'interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations')}
    values.update({name: getattr(state, name) for name in (
        'observations', 'streak', 'first_tick', 'counted_tick', 'last_tick', 'last_capture')})
    value = Window(expected_goal, owner.state.tail.hex(), owner.state.frames, **values)
    batch.budget.remaining()
    return value


def guard(owner, batch, authority, expected):
    authority.clock_guard()
    require(authority.address == batch.address, 'verification endpoint differs')
    require(witness(owner, batch, expected.goal_digest) == expected,
            'original completion history changed after review')
    batch.check()
    batch.budget.remaining()


def record_path(directory, confirmation):
    digest_text(confirmation)
    path = Path(directory)
    require(path.is_absolute() and str(path) == directory and '..' not in path.parts
            and len(directory) <= 4096, 'canonical private clock directory required')
    return path / ('progress-' + confirmation + '.run')


def encoded(operation, state=None, error=False):
    value = route.packet('completion-clock-' + operation, state, error)
    value['schema'] = 'dfmcp.completion-clock-result/1'
    if error:
        value['clock_effect_outcome_unknown'] = operation == 'advance'
        value['error'] = {'code': 'COMPLETION_CLOCK_REFUSED', 'detail':
            'Original goal, journals, frontier, consent or allowance refused. Preserve the original confirmation and query its clock record; never retry the effect.'}
    value['agent_turn']['briefing'].update(construction_completion_proven=False,
        completion_sample_appended=False, original_goal_policy_changed=False)
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete verification output exceeds bound')
    return raw


def reserve(state):
    require(len(encoded('review', state)) + outcomes.MAX_OUTPUT + 16 * 1024 <= MAX_OUTPUT,
            'verification result cannot fit before clock dispatch')


def review(owner, batch, authority, expected_goal, *, max_ticks=1200, wall_ms=1000):
    integer(max_ticks, 1, 1200)
    integer(wall_ms, 1, 60000)
    require(type(authority) is ProgressAuthority, 'closed construction-progress authority required')
    original = witness(owner, batch, expected_goal)
    guard(owner, batch, authority, original)
    state, report = final.read_frontier(batch, authority)
    reserve(state)
    decision = next_sample(original, report['source']['tick'], max_ticks=max_ticks)
    scope = None
    if report['status'] != 'construction_quiescent':
        decision = dict(decision, status='refused', reason='original_construction_not_quiescent',
                        game_ticks=None, target_tick=None)
    elif decision['status'] == 'advance_for_sample' and report['source']['inventory_sha256'] != original.last_capture:
        decision = dict(decision, status='sample_required', reason='sample_changed_capture_before_clock',
                        game_ticks=None, target_tick=None)
    if decision['status'] == 'advance_for_sample':
        with Client(authority, batch.budget,
                    lambda: guard(owner, batch, authority, original)) as client:
            observation = client.call('ObserveRun')
        require(not observation['owner_active'] and observation['retained_records'] < 256,
                'clock already owned or record capacity exhausted')
        candidate = CompletionScope.create({
            'schema': SCHEMA, 'policy': POLICY, 'phase': 'completion_verification',
            'batch_id': batch.id, 'review_digest': batch.review.digest, 'plan_digest': batch.plan.digest,
            'inventory_digest': state['inventory_digest'], 'frontier_evidence': report['evidence_digest'],
            'source': report['source'], 'game_ticks': decision['game_ticks'], 'wall_ms': wall_ms,
            'clock_manifest': observation['manifest'], 'clock_observation_hex': observation['observation_hex'],
            'completion': original.json(), 'max_ticks': max_ticks})
        after, latest = final.read_frontier(batch, authority)
        guard(owner, batch, authority, original)
        if (latest['status'] == 'construction_quiescent'
                and latest['evidence_digest'] == report['evidence_digest']
                and after['inventory_digest'] == state['inventory_digest']):
            candidate.bind(batch, current=True)
            scope = candidate
        else:
            decision = dict(decision, status='refused', reason='frontier_changed_during_verification_review',
                            game_ticks=None, target_tick=None)
        state = after
    state.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                 execution_review_required=True)
    state['completion_clock_review'] = {'schema': SCHEMA, 'policy': POLICY,
        'decision': decision, 'scope': None if scope is None else scope.value(),
        'confirm_clock': None if scope is None else scope.digest,
        'goal_digest': expected_goal, 'monitor_head': original.journal_head,
        'completion_sample_appended': False, 'construction_completion_proven': False,
        'goal_policy_changed': False}
    reserve(state)
    guard(owner, batch, authority, original)
    return state


def advance(owner, batch, authority, expected_goal, confirmation, clock_directory,
            *, max_ticks=1200, wall_ms=1000):
    """One exact clock effect; the original monitor is NEVER advanced by inference."""
    digest_text(confirmation)
    path = record_path(clock_directory, confirmation)
    initial = review(owner, batch, authority, expected_goal, max_ticks=max_ticks, wall_ms=wall_ms)
    approved = initial['completion_clock_review']
    require(approved['confirm_clock'] == confirmation, 'verification consent is stale or for another goal')
    scope = CompletionScope.create(approved['scope'])
    original = Window.from_json(scope.value()['completion'])
    reserve(initial)
    budget = batch.budget
    require(budget.calls >= 10 and budget.network_bytes >= 3 * 1024 * 1024
            and int(budget.remaining() * 1000) > wall_ms, 'verification clock cannot fit shared allowance')
    budget.reserve('disk_bytes', CLOCK_DISK)
    guard(owner, batch, authority, original)
    with outcomes.new_run(path) as parent:
        with Inventory(path.parent, batch, parent_fd=parent) as history:
            history.admit_new(scope)
            with ScopeStore(path, parent, batch, confirmation, create=scope) as binding:
                def current():
                    binding.check()
                    scope.bind(batch, current=True)
                    guard(owner, batch, authority, original)
                def effect(intent):
                    history.check()
                    binding.verify_intent(intent)
                    current()
                    history.check()
                with Client(authority, budget, current, intent=scope.intent(), effect_guard=effect) as client:
                    result = clock.start(client, authority.address, path,
                                         scope.clock_key, scope.value()['game_ticks'], wall_ms)
                with outcomes.OutcomeStore(path, parent_fd=parent) as store:
                    require(store.intent == scope.intent(), 'retained verification intent differs')
                    retained = store.retain(result)
                    output = batch.audit()
                    output.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                                  execution_review_required=True)
                    output['completion_clock'] = {'schema': SCHEMA, 'policy': POLICY,
                        'scope': scope.value(), 'clock_record_id': scope.digest, 'clock_key': scope.clock_key,
                        'clock_result': retained, 'target_tick': approved['decision']['target_tick'],
                        'goal_digest': expected_goal, 'original_monitor_head': original.journal_head,
                        'prior_clock_inventory_digest': history.digest,
                        'clock_commit_invoked_this_call': client.commit_attempted,
                        'completion_sample_appended': False, 'fresh_completion_sample_required': True,
                        'construction_completion_proven': False, 'original_goal_policy_changed': False,
                        'automatic_second_clock_burst': False, 'placement_dispatched': False,
                        'current_pause_unproved': True, 'retry_permitted': False}
                    encoded('advance', output)
                    current()
                    store.check()
                    history.check()
                    return output


class Parser(argparse.ArgumentParser):
    def error(self, _message):
        raise ValueError('invalid completion verification arguments')


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyboardInterrupt):
        return 2


def main(argv=None):
    operation, attempted = 'unknown', False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('review', 'advance'))
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--completion-journal', required=True)
        parser.add_argument('--expected-goal', required=True)
        parser.add_argument('--clock-directory')
        parser.add_argument('--confirm-clock')
        parser.add_argument('--max-ticks', type=int, default=1200)
        parser.add_argument('--wall-ms', type=int, default=1000)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        for value in (args.batch_id, args.expected_goal): digest_text(value)
        integer(args.max_ticks, 1, 1200)
        integer(args.wall_ms, 1, 60000)
        require(all((value is not None) == (operation == 'advance')
                    for value in (args.clock_directory, args.confirm_clock)),
                'only advance selects a private clock directory and confirmation')
        origin, journal = Path(args.directory), Path(args.completion_journal)
        require(origin.is_absolute() and journal.is_absolute() and not journal.is_relative_to(origin),
                'completion journal must be outside original batch custody')
        if operation == 'advance':
            path = record_path(args.clock_directory, args.confirm_clock)
            require(not path.parent.is_relative_to(origin) and not journal.is_relative_to(path.parent),
                    'clock custody must be separate from original batch and monitor')
        budget = make_budget(args.timeout_ms)
        authority = ProgressAuthority.load()
        with ExitStack() as custody:
            batch = Batch(args.directory, args.batch_id, budget, writable=False)
            custody.callback(batch.close)
            owner = Journal(args.completion_journal, budget, writable=False)
            custody.callback(owner.close)
            state = (review(owner, batch, authority, args.expected_goal,
                            max_ticks=args.max_ticks, wall_ms=args.wall_ms) if operation == 'review' else
                     advance(owner, batch, authority, args.expected_goal, args.confirm_clock,
                             args.clock_directory, max_ticks=args.max_ticks, wall_ms=args.wall_ms))
            raw = encoded(operation, state)
            authority.clock_guard()
            owner.check()
            batch.check()
            budget.remaining()
            attempted = True
            return publish(raw)
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not attempted:
            publish(encoded(operation, error=True))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
