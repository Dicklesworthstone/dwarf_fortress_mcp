#!/usr/bin/env python3
"""Explicit bounded clock progress after every original furniture placement.

This owner closes the execution gap between the final sampled construction
frontier and the existing run/1.13 clock effect. One independently confirmed
all-placed frontier authorizes at most one bounded clock burst. It never places
furniture, retries placement, loops automatically, or upgrades sampled
quiescence into stability-qualified construction completion.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import sys

import bounded_run_client as clock
import bounded_run_outcomes as outcomes
from furniture_plan import canonical, integer, require
import route_furniture_batch as route
from route_furniture_batch_store import Batch
from route_furniture_clock_context import Client, FINAL_POLICY, FINAL_SCHEMA, FinalScope, ScopeStore
from route_furniture_clock_inventory import Inventory
import route_furniture_final as final
from route_furniture_progress import ProgressAuthority, RUN_ENV
from route_furniture_review import Source, digest_text

MAX_OUTPUT = 256 * 1024
CLOCK_CALL_RESERVE = 10
CLOCK_NETWORK_RESERVE = 3 * 1024 * 1024
CLOCK_DISK_RESERVE = 256 * 1024


def _record_path(directory: str, confirmation: str) -> Path:
    digest_text(confirmation)
    base = Path(directory)
    require(base.is_absolute() and '..' not in base.parts and len(str(base)) <= 4096,
            'final clock directory must be absolute without parent traversal')
    return base / ('progress-' + confirmation + '.run')


def _reserve(state):
    require(len(encoded('review', state)) + outcomes.MAX_OUTPUT + outcomes.MAX_FILE <= MAX_OUTPUT,
            'complete final-progress result cannot fit before dispatch')


def _frontier(batch, authority):
    require(not batch.writable, 'final progress holds original batch custody read-only')
    state, report = final.read_frontier(batch, authority)
    require(report['batch_id'] == batch.id and report['phase'] == 'final_construction',
            'final progress review belongs to another batch or phase')
    return state, report


def _scope(batch, state, report, ticks, wall_ms, observation):
    if report['status'] != 'construction_pending':
        return None
    require(not observation['owner_active'] and observation['retained_records'] < 256,
            'clock already owned or retained-record capacity exhausted')
    return FinalScope.create({
        'schema': FINAL_SCHEMA, 'policy': FINAL_POLICY, 'phase': 'final_construction',
        'batch_id': batch.id, 'review_digest': batch.review.digest,
        'plan_digest': batch.plan.digest, 'inventory_digest': state['inventory_digest'],
        'frontier_evidence': report['evidence_digest'], 'source': report['source'],
        'game_ticks': ticks, 'wall_ms': wall_ms,
        'clock_manifest': observation['manifest'],
        'clock_observation_hex': observation['observation_hex']})


def review_final_progress(batch, authority: ProgressAuthority, ticks: int, wall_ms: int):
    """Review one exact pending final frontier around a read-only clock observation."""
    integer(ticks, 1, 1200)
    integer(wall_ms, 1, 60000)
    require(type(authority) is ProgressAuthority and not batch.writable,
            'final clock review requires current read-only progress authority')
    authority.clock_guard()
    state, report = _frontier(batch, authority)
    _reserve(state)
    scope, reason = None, 'final_frontier_not_construction_pending'
    if report['status'] == 'construction_pending':
        inventory = state['inventory_digest']

        def guard():
            batch.check()
            require(batch.audit()['inventory_digest'] == inventory,
                    'original final batch changed during clock review')

        with Client(authority, batch.budget, guard) as client:
            observed = client.call('ObserveRun')
        candidate = _scope(batch, state, report, ticks, wall_ms, observed)
        after, latest = _frontier(batch, authority)
        if (latest['status'] == 'construction_pending'
                and latest['evidence_digest'] == report['evidence_digest']
                and after['inventory_digest'] == inventory):
            candidate.bind(batch, current=True)
            scope, reason = candidate, 'exact_clock_and_pending_final_frontier_reviewed'
        else:
            reason = 'final_frontier_changed_during_clock_review'
        state = after
    state['final_progress_review'] = {
        'schema': FINAL_SCHEMA, 'policy': FINAL_POLICY,
        'scope': None if scope is None else scope.value(),
        'confirm_progress': None if scope is None else scope.digest,
        'reason': reason, 'clock_mutation_performed': False,
        'automatic_loop_permitted': False, 'placement_permitted': False,
        'construction_completion_proven': False,
        'atomic_cross_profile_snapshot_proven': False}
    _reserve(state)
    authority.clock_guard()
    batch.check()
    batch.budget.remaining()
    return state


def _post_review(batch, authority, scope, retained, requested):
    record = retained.get('record', {})
    verified = record.get('phase') == 'stopped' and record.get('pause_verified') is True
    if not requested or not verified:
        return None, 'not_requested' if not requested else 'clock_stop_unestablished'
    require(authority is not None, 'fresh final construction inspection requires authority')
    selected = scope.value()
    original = Source.from_json(selected['source'])
    tick = record.get('observed_tick')
    if (tick is None or tick < original.tick
            or retained['manifest']['generation'] != selected['clock_manifest']['generation']):
        return None, 'clock_stop_source_unestablished'
    try:
        after, report = _frontier(batch, authority)
        current = Source.from_json(report['source'])
        original.permits(current)
        require(current.tick >= tick, 'final construction observation predates stopped clock effect')
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        authority.clock_guard()
        batch.check()
        batch.budget.remaining()
        return None, 'final_readiness_unestablished'
    return {
        'status': report['status'], 'frontier_evidence': report['evidence_digest'],
        'construction_status': report['construction_prefix']['status'],
        'all_original_furnishings_quiescent_now':
            report['all_original_furnishings_quiescent_now'],
        'game_tick': report['source']['tick']}, 'observed_after_clock_stop'


def _result(batch, authority, scope, retained, *, contacted, committed, refresh):
    after, status = _post_review(batch, authority, scope, retained, refresh)
    output = batch.audit()
    output.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                  execution_review_required=True)
    record = retained.get('record', {})
    output['final_construction_progress'] = {
        'schema': FINAL_SCHEMA, 'policy': FINAL_POLICY, 'scope': scope.value(),
        'clock_key': scope.clock_key, 'clock_record_id': scope.digest,
        'clock_result': retained,
        'clock_terminal_pause_verified':
            record.get('phase') == 'stopped' and record.get('pause_verified') is True,
        'current_pause_unproved': True,
        'native_query_contacted': contacted and not committed,
        'clock_commit_invoked_this_call': committed,
        'post_progress_review': after, 'post_progress_review_status': status,
        'all_original_furnishings_quiescent_now':
            bool(after and after['all_original_furnishings_quiescent_now']),
        'automatic_second_clock_burst': False, 'placement_dispatched': False,
        'placement_retry_permitted': False, 'construction_completion_proven': False}
    encoded('progress', output)
    if authority is not None:
        authority.clock_guard()
    batch.check()
    batch.budget.remaining()
    return output


def progress_final(batch, authority: ProgressAuthority, ticks: int, wall_ms: int,
                   confirmation: str, clock_directory: str):
    """Execute at most one confirmed final-construction clock effect."""
    digest_text(confirmation)
    integer(ticks, 1, 1200)
    integer(wall_ms, 1, 60000)
    require(type(authority) is ProgressAuthority and not batch.writable,
            'final clock progress requires read-only original custody')
    path = _record_path(clock_directory, confirmation)
    initial = review_final_progress(batch, authority, ticks, wall_ms)
    require(initial['final_progress_review']['confirm_progress'] == confirmation,
            'final progress confirmation differs from current construction frontier')
    scope = FinalScope.create(initial['final_progress_review']['scope'])
    budget = batch.budget
    require(budget.calls >= CLOCK_CALL_RESERVE
            and budget.network_bytes >= CLOCK_NETWORK_RESERVE,
            'shared allowance cannot admit complete final clock start')
    budget.reserve('disk_bytes', CLOCK_DISK_RESERVE)
    require(int(budget.remaining() * 1000) > wall_ms,
            'shared foreground deadline cannot contain final bounded run')
    authority.clock_guard()
    with outcomes.new_run(path) as parent:
        with Inventory(path.parent, batch, parent_fd=parent) as history:
            history.admit_new(scope)
            with ScopeStore(path, parent, batch, confirmation, create=scope) as binding:
                def guard():
                    binding.check()
                    scope.bind(batch, current=True)

                def effect_guard(intent):
                    history.check()
                    binding.verify_intent(intent)
                    history.check()

                with Client(authority, budget, guard, intent=scope.intent(),
                            effect_guard=effect_guard) as client:
                    result = clock.start(client, authority.address, path,
                                         scope.clock_key, ticks, wall_ms)
                with outcomes.OutcomeStore(path, parent_fd=parent) as store:
                    require(store.intent == scope.intent(),
                            'retained final intent differs from batch association')
                    retained = store.retain(result)
                    output = _result(batch, authority, scope, retained,
                                     contacted=True, committed=client.commit_attempted,
                                     refresh=True)
                    output['final_construction_progress']['prior_clock_inventory_digest'] = history.digest
                    encoded('progress', output)
                    binding.check()
                    store.check()
                    history.check()
                    return output


def query_final_progress(batch, authority: ProgressAuthority | None, confirmation: str,
                         clock_directory: str, wait_ms: int = 0, *, refresh_frontier=False):
    """Query-only recovery of one original final-construction clock effect."""
    digest_text(confirmation)
    integer(wait_ms, 0, 60000)
    require(type(refresh_frontier) is bool and not batch.writable,
            'final clock recovery needs read-only original custody')
    require(authority is None or (type(authority) is ProgressAuthority
            and authority.address == batch.address),
            'final clock recovery authority belongs to another source')
    require(not refresh_frontier or authority is not None,
            'fresh final construction inspection requires authority')
    path = _record_path(clock_directory, confirmation)
    with outcomes.OutcomeStore(path) as store:
        with ScopeStore(path, store.parent, batch, confirmation) as binding:
            scope = binding.scope
            require(type(scope) is FinalScope,
                    'selected clock record is not a final-construction effect')
            require(store.intent == scope.intent(),
                    'final clock intent belongs to another scoped effect')
            retained, contacted = store.load(), False
            if retained is None:
                require(type(authority) is ProgressAuthority,
                        'native final recovery requires current clock authority')
                batch.budget.reserve('disk_bytes', CLOCK_DISK_RESERVE)

                def guard():
                    binding.check()
                    batch.budget.reserve('disk_bytes', 2 * outcomes.MAX_FILE)
                    store.check()

                with Client(authority, batch.budget, guard,
                            intent=store.intent, query=True) as client:
                    result = clock.recover(client, store.intent, False, wait_ms)
                retained = store.retain(result)
                contacted = True
            else:
                retained = store.view()
            output = _result(batch, authority, scope, retained,
                             contacted=contacted, committed=False,
                             refresh=refresh_frontier)
            binding.check()
            store.check()
            return output


def packet(operation, state=None, error=False):
    value = route.packet('final-progress-' + operation, state, error)
    value['schema'] = 'dfmcp.route-furniture-final-progress-result/1'
    if state is not None and 'final_construction_progress' in state:
        report = state['final_construction_progress']
        value['agent_turn']['briefing'].update(
            final_clock_commit_invoked_this_call=report['clock_commit_invoked_this_call'],
            all_original_furnishings_quiescent_now=
                report['all_original_furnishings_quiescent_now'],
            construction_completion_proven=False)
    return value


def encoded(operation, state=None, error=False):
    raw = canonical(packet(operation, state, error)) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete final-progress packet exceeds output bound')
    return raw


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyboardInterrupt):
        return 2


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid final-construction progress arguments')


def main(argv=None):
    batch, operation, output_attempted = None, 'unknown', False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('review', 'progress', 'query'))
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--ticks', type=int)
        parser.add_argument('--wall-ms', type=int)
        parser.add_argument('--confirm-progress')
        parser.add_argument('--clock-directory')
        parser.add_argument('--wait-ms', type=int)
        parser.add_argument('--refresh-frontier', action='store_true')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        require(all((v is not None) == (operation in ('review', 'progress'))
                    for v in (args.ticks, args.wall_ms)),
                'ticks and wall-ms belong only to final review/progress')
        require(all((v is not None) == (operation in ('progress', 'query'))
                    for v in (args.confirm_progress, args.clock_directory)),
                'final progress/query require confirmation and private clock directory')
        require(operation == 'query' or (args.wait_ms is None and not args.refresh_frontier),
                'wait and refresh belong only to final query')
        digest_text(args.batch_id)
        if operation != 'query':
            integer(args.ticks, 1, 1200)
            integer(args.wall_ms, 1, 60000)
        if args.confirm_progress is not None:
            _record_path(args.clock_directory, args.confirm_progress)
        wait_ms = 0 if args.wait_ms is None else integer(args.wait_ms, 0, 60000)
        budget = route.Budget(args.timeout_ms)
        authority = (None if operation == 'query' and not args.refresh_frontier
                     and not any(key in os.environ for key in RUN_ENV)
                     else ProgressAuthority.load())
        batch = Batch(args.directory, args.batch_id, budget, writable=False)
        state = (review_final_progress(batch, authority, args.ticks, args.wall_ms)
                 if operation == 'review' else
                 progress_final(batch, authority, args.ticks, args.wall_ms,
                                args.confirm_progress, args.clock_directory)
                 if operation == 'progress' else
                 query_final_progress(batch, authority, args.confirm_progress,
                                      args.clock_directory, wait_ms,
                                      refresh_frontier=args.refresh_frontier))
        raw = encoded(operation, state)
        if authority is not None:
            authority.clock_guard()
        batch.check()
        budget.remaining()
        output_attempted = True
        return publish(raw)
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not output_attempted:
            publish(encoded(operation, None, True))
        return 2
    finally:
        if batch is not None:
            batch.close()


if __name__ == '__main__':
    raise SystemExit(main())
