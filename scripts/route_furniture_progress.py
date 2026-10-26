#!/usr/bin/env python3
"""Dwarf Fortress game-clock progress for original furniture construction.

One independently confirmed, exact-clock-bound burst; no automatic placement,
clock loop or blind retry. Native run/1.13 and its intent/receipt codecs remain
unchanged. The immutable batch association is separate from native authority.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import os
from pathlib import Path
import sys

import bounded_run_client as clock
import bounded_run_outcomes as outcomes
from furniture_plan import canonical, integer, require
import route_furniture_batch as route
import route_furniture_run as runner
from route_furniture_review import Source, digest_text
from route_furniture_batch_store import Batch
from route_furniture_clock_context import Client, POLICY, SCHEMA, Scope, FinalScope, ScopeStore
from route_furniture_clock_inventory import Inventory

MAX_OUTPUT = 256 * 1024
# Admission thresholds only; actual native frames/bytes consume the SAME budget.
CLOCK_CALL_RESERVE = 10
CLOCK_NETWORK_RESERVE = 3 * 1024 * 1024
CLOCK_DISK_RESERVE = 256 * 1024
RUN_ENV = ('DFMCP_ALLOW_UNADMITTED_RUN_V1_13', 'DFMCP_RUN_ENDPOINT', 'DFMCP_RUN_TOKEN')
ENVIRONMENT = tuple(route.ENVIRONMENT) + RUN_ENV


@dataclass(frozen=True)
class ProgressAuthority(route.Authority):
    run_token: bytes = field(repr=False, default=b'')

    @classmethod
    def load(cls):
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'progress requires its isolated route-plus-run environment')
        require(os.environ.get(route.OPT_IN) == '1', 'route-batch development opt-in required')
        require(os.environ.get('DFMCP_ALLOW_UNADMITTED_RUN_V1_13') == '1',
                'bounded-run development opt-in required')
        require(os.environ.get(route.PLACE) in (None, '0'),
                'construction progress refuses furniture placement permission')
        tokens = tuple(os.environ.get(key, '').encode('utf-8')
                       for key in (route.native.TOKEN, route.OPS_TOKEN, route.MAP_TOKEN, 'DFMCP_RUN_TOKEN'))
        require(all(32 <= len(value) <= 256 and b'\0' not in value for value in tokens),
                'invalid progress credentials')
        address = route.native.endpoint(os.environ.get(route.ENDPOINT, '127.0.0.1:5000'))
        raw_endpoint = os.environ.get('DFMCP_RUN_ENDPOINT', '127.0.0.1:5000')
        require(address == clock.endpoint(raw_endpoint)
                and raw_endpoint == f'{address[0]}:{address[1]}',
                'route and bounded-run endpoints must be identical and canonical')
        return cls(address, *tokens[:3], False, tokens[3])

    def guard(self, operation: str = 'QueryPlacement') -> None:
        require(operation in ('Handshake', 'ReadPlacement', 'QueryPlacement'),
                'progress cannot acquire furniture mutation authority')
        require(ProgressAuthority.load() == self, 'progress endpoint or credentials changed')

    def clock_guard(self) -> None:
        require(ProgressAuthority.load() == self, 'bounded-run authority changed')


def _frontier(batch, authority):
    require(not batch.writable, 'progress holds original route custody read-only')
    before = batch.audit()
    state = route.review_step(batch, authority)
    runner._review_status(batch, before, state)
    report = state['route_frontier']
    require(report['batch_id'] == batch.id, 'progress review belongs to another batch')
    return state, report


def _record_path(directory: str, confirmation: str) -> Path:
    digest_text(confirmation)
    base = Path(directory)
    require(base.is_absolute() and '..' not in base.parts and len(str(base)) <= 4096,
            'clock record directory must be absolute without parent traversal')
    return base / ('progress-' + confirmation + '.run')


def _clock_key(confirmation: str) -> str:
    digest_text(confirmation)
    return 'frp-' + confirmation


def _reserve(state):
    require(len(encoded('review', state)) + outcomes.MAX_OUTPUT + outcomes.MAX_FILE <= MAX_OUTPUT,
            'complete scoped clock result cannot fit before dispatch')


def _scope(batch, state, report, ticks, wall_ms, observation):
    if report['status'] != 'construction_pending':
        return None
    require(not observation['owner_active'] and observation['retained_records'] < 256,
            'clock already owned or retained-record capacity exhausted')
    return Scope.create({'schema': SCHEMA, 'policy': POLICY, 'batch_id': batch.id,
        'review_digest': batch.review.digest, 'plan_digest': batch.plan.digest,
        'inventory_digest': state['inventory_digest'], 'next_step': state['next_step'],
        'frontier_evidence': report['evidence_digest'], 'source': report['source'],
        'game_ticks': ticks, 'wall_ms': wall_ms, 'clock_manifest': observation['manifest'],
        'clock_observation_hex': observation['observation_hex']})


def review_progress(batch, authority, ticks: int, wall_ms: int):
    """Read-only approval over equal pending route endpoints around ObserveRun.

    Equal endpoints are not an atomic cross-profile snapshot. The exact clock
    observation is also revalidated by the original native prepare/commit path.
    """
    integer(ticks, 1, 1200)
    integer(wall_ms, 1, 60000)
    require(type(authority) is ProgressAuthority and not batch.writable,
            'clock-aware review requires read-only original custody')
    authority.clock_guard()
    if batch.audit()['status'] == 'all_placed':
        # All registered placements have no next item to review. Use the full
        # original-receipt frontier, not an invented next step or empty prefix.
        import route_furniture_final_progress as final_progress
        state = final_progress.review_final_progress(batch, authority, ticks, wall_ms)
        state['progress_review'] = dict(state['final_progress_review'])
        _reserve(state)  # Include the common handoff in the complete byte bound.
        authority.clock_guard()
        batch.check()
        batch.budget.remaining()
        return state
    state, report = _frontier(batch, authority)
    _reserve(state)
    scope, reason = None, 'frontier_not_construction_pending'
    if report['status'] == 'construction_pending':
        def guard():
            batch.check()
            require(batch.audit()['inventory_digest'] == state['inventory_digest'],
                    'batch inventory changed during clock review')
        with Client(authority, batch.budget, guard) as client:
            observed = client.call('ObserveRun')
        candidate = _scope(batch, state, report, ticks, wall_ms, observed)
        after, latest = _frontier(batch, authority)
        if (latest['status'] == 'construction_pending'
                and latest['evidence_digest'] == report['evidence_digest']
                and after['inventory_digest'] == state['inventory_digest']):
            candidate.bind(batch, current=True)
            scope, reason = candidate, 'exact_clock_and_pending_frontier_reviewed'
        else:
            reason = 'frontier_changed_during_clock_review'
        state = after
    state['progress_review'] = {'policy': POLICY, 'scope': None if scope is None else scope.value(),
        'confirm_progress': None if scope is None else scope.digest, 'reason': reason,
        'clock_mutation_performed': False, 'automatic_loop_permitted': False,
        'placement_permitted': False, 'atomic_cross_profile_snapshot_proven': False}
    _reserve(state)
    authority.clock_guard()
    batch.check()
    batch.budget.remaining()
    return state


def _post_review(batch, authority, scope, retained, requested):
    verified = (retained.get('record', {}).get('phase') == 'stopped'
                and retained['record'].get('pause_verified') is True)
    if not requested or not verified:
        return None, 'not_requested' if not requested else 'clock_stop_unestablished'
    require(authority is not None, 'fresh route inspection requires current authority')
    selected = scope.value()
    original = Source.from_json(selected['source'])
    tick = retained['record'].get('observed_tick')
    if (tick is None or tick < original.tick
            or retained['manifest']['generation'] != selected['clock_manifest']['generation']):
        return None, 'clock_stop_source_unestablished'
    try:
        after, report = _frontier(batch, authority)
        current = Source.from_json(report['source'])
        original.permits(current)
        require(current.tick >= tick, 'post-clock route observation predates the stopped clock effect')
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        # Keep the already retained clock outcome separate from failed current
        # readiness. Never repeat this acquisition or reuse the pre-clock read.
        authority.clock_guard()
        batch.check()
        batch.budget.remaining()
        return None, 'readiness_unestablished'
    return {'status': report['status'], 'frontier_evidence': report['evidence_digest'],
            'construction_status': report['construction_prefix']['status'],
            'next_step': after['next_step'], 'expected_plan': after['expected_plan'],
            'confirm_review': after['confirm_review']}, 'observed_after_clock_stop'


def _result(batch, authority, scope, retained, *, contacted, committed, refresh):
    # Recovery follows the retained scope, NEVER today's placement count. An
    # old next-step receipt remains next-step history after the batch finishes.
    if type(scope) is FinalScope:
        import route_furniture_final_progress as final_progress
        return final_progress._result(batch, authority, scope, retained,
            contacted=contacted, committed=committed, refresh=refresh)
    require(type(scope) is Scope, 'unsupported retained progress scope')
    after, status = _post_review(batch, authority, scope, retained, refresh)
    output = batch.audit()
    # Original journal eligibility is not a fresh execution confirmation.
    output.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                  execution_review_required=True)
    record = retained.get('record', {})
    output['construction_progress'] = {'schema': SCHEMA, 'policy': POLICY, 'scope': scope.value(),
        'clock_key': _clock_key(scope.digest), 'clock_record_id': scope.digest,
        'clock_result': retained, 'clock_terminal_pause_verified':
            record.get('phase') == 'stopped' and record.get('pause_verified') is True,
        'current_pause_unproved': True, 'native_query_contacted': contacted and not committed,
        'clock_commit_invoked_this_call': committed,
        'post_progress_review': after, 'post_progress_review_status': status,
        'next_step_ready': bool(after and after['status'] == 'ready'),
        'automatic_second_clock_burst': False, 'placement_dispatched': False,
        'placement_retry_permitted': False, 'construction_completion_proven': False}
    encoded('progress', output)
    if authority is not None:
        authority.clock_guard()
    batch.check()
    batch.budget.remaining()
    return output


def progress(batch, authority: ProgressAuthority, ticks: int, wall_ms: int,
             confirmation: str, clock_directory: str):
    """One independently confirmed effect with association/intent/receipt custody."""
    digest_text(confirmation)
    require(not batch.writable and type(authority) is ProgressAuthority,
            'clock progress requires read-only original custody and current authority')
    integer(ticks, 1, 1200)
    integer(wall_ms, 1, 60000)
    path = _record_path(clock_directory, confirmation)
    authority.clock_guard()
    if batch.audit()['status'] == 'all_placed':
        # The final owner independently reacquires and validates final consent;
        # a historical next-step confirmation is not converted or auto-approved.
        import route_furniture_final_progress as final_progress
        return final_progress.progress_final(batch, authority, ticks, wall_ms,
                                              confirmation, clock_directory)
    initial = review_progress(batch, authority, ticks, wall_ms)
    require(initial['progress_review']['confirm_progress'] == confirmation,
            'progress confirmation differs from the current clock or construction frontier')
    scope = Scope.create(initial['progress_review']['scope'])
    _reserve(initial)
    budget = batch.budget
    require(budget.calls >= CLOCK_CALL_RESERVE and budget.network_bytes >= CLOCK_NETWORK_RESERVE,
            'shared allowance cannot admit the complete clock start')
    # Existing native capsule/outcome I/O is bounded internally. Reserve a fixed
    # allowance for it; scope checks and all network I/O are charged separately.
    budget.reserve('disk_bytes', CLOCK_DISK_RESERVE)
    require(int(budget.remaining() * 1000) > wall_ms,
            'shared foreground deadline cannot contain the bounded run')
    authority.clock_guard()
    with outcomes.new_run(path) as parent:
        with Inventory(path.parent, batch, parent_fd=parent) as history:
            # A new confirmation must not bypass an unresolved older effect.
            # Read/verify the whole directory, never just a filtered output page.
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
                                         _clock_key(confirmation), ticks, wall_ms)
                # Borrow the original directory lock instead of releasing it between
                # native completion and original terminal evidence publication.
                with outcomes.OutcomeStore(path, parent_fd=parent) as store:
                    require(store.intent == scope.intent(), 'retained intent differs from batch association')
                    retained = store.retain(result)
                    output = _result(batch, authority, scope, retained, contacted=True,
                                     committed=client.commit_attempted, refresh=True)
                    output['construction_progress']['prior_clock_inventory_digest'] = history.digest
                    encoded('progress', output)
                    binding.check()
                    store.check()
                    history.check()
                    return output


def query_progress(batch, authority: ProgressAuthority | None, confirmation: str,
                   clock_directory: str, wait_ms: int = 0, *, refresh_frontier=False):
    """Query-only original recovery; terminal inspection is offline by default.

    A source-bound scope is required to attribute an old clock effect to this
    batch. Older unassociated records stay queryable through bounded_run_client,
    not through an invented batch association. Refresh is explicitly separate.
    """
    digest_text(confirmation)
    integer(wait_ms, 0, 60000)
    require(type(refresh_frontier) is bool and not batch.writable,
            'clock recovery needs read-only original custody and explicit refresh')
    require(authority is None or (type(authority) is ProgressAuthority
            and authority.address == batch.address), 'clock recovery authority belongs to another source')
    require(not refresh_frontier or authority is not None, 'fresh route inspection requires authority')
    path = _record_path(clock_directory, confirmation)
    with outcomes.OutcomeStore(path) as store:
        with ScopeStore(path, store.parent, batch, confirmation) as binding:
            scope = binding.scope
            require(store.intent == scope.intent(), 'clock intent belongs to another scoped effect')
            retained, contacted = store.load(), False
            if retained is None:
                require(type(authority) is ProgressAuthority, 'native recovery requires current clock authority')
                batch.budget.reserve('disk_bytes', CLOCK_DISK_RESERVE)
                def guard():
                    binding.check()
                    batch.budget.reserve('disk_bytes', 2 * outcomes.MAX_FILE)
                    store.check()
                with Client(authority, batch.budget, guard, intent=store.intent, query=True) as client:
                    result = clock.recover(client, store.intent, False, wait_ms)
                retained = store.retain(result)
                contacted = True
            else:
                retained = store.view()
            output = _result(batch, authority, scope, retained, contacted=contacted,
                             committed=False, refresh=refresh_frontier)
            binding.check()
            store.check()
            return output


def inventory_state(batch, history: Inventory, *, selection='all', limit=16, continuation=None):
    """Offline handoff of original clock identities; no execution permission."""
    page = history.page(selection=selection, limit=limit, continuation=continuation)
    state = batch.audit()
    state.update(advance_allowed=False, expected_plan=None, confirm_review=None,
                 execution_review_required=True, clock_inventory=page)
    encoded('inventory', state)
    history.check()
    return state


def packet(operation, state=None, error=False):
    value = route.packet('progress-' + operation, state, error)
    value['schema'] = 'dfmcp.route-furniture-progress-result/2'
    if state is not None and 'clock_inventory' in state:
        inventory = state['clock_inventory']
        turn = value['agent_turn']
        turn['phase'] = 'inspect'
        turn['briefing'].update(clock_records=inventory['counts']['records'],
                               unresolved_clocks=inventory['counts']['unresolved'],
                               clock_start_authorized=False)
        turn['coverage']['clock_history_directory_validated'] = True
        turn['coverage']['clock_history_other_batch_details_omitted'] = True
        turn['active_work'] = list(turn['active_work']) + [
            {'kind': 'construction_clock', 'clock_record_id': row['clock_record_id'],
             'status': row['status'], 'query_supported': row['query_supported'],
             'retry_permitted': False}
            for row in inventory['rows'] if not row['resolved']]
        turn['uncertainty'].append(
            'Clock history covers only the selected private directory. It does not prove current pause, global absence, construction completion or permission to retry. Scope-only and source-loss records require original-custody investigation.')
    if state is not None and 'construction_progress' in state:
        progress = state['construction_progress']
        value['agent_turn']['briefing'].update(
            clock_commit_invoked_this_call=progress['clock_commit_invoked_this_call'],
            next_step_ready=progress['next_step_ready'], construction_completion_proven=False)
    if state is not None and 'final_construction_progress' in state:
        progress = state['final_construction_progress']
        value['agent_turn']['briefing'].update(
            clock_commit_invoked_this_call=progress['clock_commit_invoked_this_call'],
            all_original_furnishings_quiescent_now=progress['all_original_furnishings_quiescent_now'],
            construction_completion_proven=False)
    return value


def encoded(operation, state=None, error=False):
    raw = canonical(packet(operation, state, error)) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete progress packet exceeds output bound')
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
        raise ValueError('invalid construction-progress arguments')


def main(argv=None):
    batch, history, operation, output_attempted = None, None, 'unknown', False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('review', 'progress', 'query', 'inventory'))
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--ticks', type=int)
        parser.add_argument('--wall-ms', type=int)
        parser.add_argument('--confirm-progress')
        parser.add_argument('--clock-directory')
        parser.add_argument('--selection', choices=('all', 'pending', 'terminal'))
        parser.add_argument('--limit', type=int)
        parser.add_argument('--continuation')
        parser.add_argument('--wait-ms', type=int)
        parser.add_argument('--refresh-frontier', action='store_true')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        require(all((v is not None) == (operation in ('review', 'progress'))
                    for v in (args.ticks, args.wall_ms)), 'ticks and wall-ms belong only to review/progress')
        require((args.confirm_progress is not None) == (operation in ('progress', 'query')),
                'only progress/query select an exact original confirmation')
        require((args.clock_directory is not None) == (operation in ('progress', 'query', 'inventory')),
                'progress/query/inventory require a private clock directory')
        require(operation == 'inventory' or all(v is None for v in (
            args.selection, args.limit, args.continuation)), 'paging options belong only to inventory')
        page_limit = 16 if args.limit is None else integer(args.limit, 1, 64)
        if operation == 'inventory':
            directory = Path(args.clock_directory)
            require(directory.is_absolute() and '..' not in directory.parts
                    and len(str(directory)) <= 4096, 'absolute private clock directory required')
            require(args.continuation is None or len(args.continuation) <= 2048,
                    'clock-history continuation exceeds bound')
        require(operation == 'query' or (args.wait_ms is None and not args.refresh_frontier),
                'wait and refresh belong only to query')
        digest_text(args.batch_id)
        if operation in ('review', 'progress'):
            integer(args.ticks, 1, 1200)
            integer(args.wall_ms, 1, 60000)
        if args.confirm_progress is not None:
            _record_path(args.clock_directory, args.confirm_progress)
        wait_ms = 0 if args.wait_ms is None else integer(args.wait_ms, 0, 60000)
        budget = route.Budget(args.timeout_ms)
        # Offline terminal query needs no credentials or native connection.
        authority = (None if operation == 'inventory' or (operation == 'query'
                     and not args.refresh_frontier and not any(key in os.environ for key in RUN_ENV))
                     else ProgressAuthority.load())
        batch = Batch(args.directory, args.batch_id, budget, writable=False)
        if operation == 'inventory':
            # Keep both directory and original batch custody through stdout.
            history = Inventory(Path(args.clock_directory), batch).__enter__()
            state = inventory_state(batch, history, selection=args.selection or 'all',
                                    limit=page_limit, continuation=args.continuation)
        else:
            state = (review_progress(batch, authority, args.ticks, args.wall_ms)
                     if operation == 'review' else
                     progress(batch, authority, args.ticks, args.wall_ms,
                              args.confirm_progress, args.clock_directory)
                     if operation == 'progress' else
                     query_progress(batch, authority, args.confirm_progress, args.clock_directory,
                                    wait_ms, refresh_frontier=args.refresh_frontier))
        raw = encoded(operation, state)
        if history is not None:
            history.check()
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
        if history is not None:
            history.close()
        if batch is not None:
            batch.close()


if __name__ == '__main__':
    # The final owner imports this module's exact ProgressAuthority class. Keep
    # direct script execution from loading a second, incompatible class copy.
    sys.modules['route_furniture_progress'] = sys.modules[__name__]
    raise SystemExit(main())