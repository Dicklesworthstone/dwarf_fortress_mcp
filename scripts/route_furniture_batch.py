#!/usr/bin/env python3
"""Place an original route-constrained furniture batch, one reviewed step at a time.

Separate unadmitted operator workflow. Reuse the existing furniture/1.19
coordinator, strict wire and journals; no new native method or replay permit.
Route checks are mandatory foreground gates, not an atomic native route lock.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-franken-storage-mvcc-54h.4.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import hashlib
import os
import stat
import sys
import time

import build_placement_client as placement
import build_placement_rpc as native
from build_placement_wire import Plan, Selection, KINDS
from furniture_plan import canonical, integer, require
from route_furniture_review import Review, Source, digest_text
import route_furniture_frontier as frontier
from route_furniture_batch_store import Batch

OPT_IN = 'DFMCP_ALLOW_UNADMITTED_ROUTE_FURNITURE_BATCH'
ENDPOINT = 'DFMCP_ROUTE_FURNITURE_BATCH_ENDPOINT'
PLACE = 'DFMCP_ROUTE_FURNITURE_ALLOW_PLACE'
MAP_TOKEN = 'DFMCP_MAP_TOKEN'
OPS_TOKEN = 'DFMCP_OPERATIONS_PAGED_TOKEN'
ENVIRONMENT = (OPT_IN, ENDPOINT, PLACE, native.TOKEN, MAP_TOKEN, OPS_TOKEN)
MAX_OUTPUT = 256 * 1024
ANALYSIS_RESERVATION = 6_000_000


@dataclass(frozen=True)
class Authority:
    address: tuple[str, int]
    token: bytes = field(repr=False)
    operations_token: bytes = field(repr=False)
    map_token: bytes = field(repr=False)
    allow_place: bool = False

    @classmethod
    def load(cls, placement: bool = False):
        require(all(not key.startswith('DFMCP_') or key in ENVIRONMENT for key in os.environ),
                'route batch requires its isolated operator environment')
        require(os.environ.get(OPT_IN) == '1', 'explicit route batch development opt-in required')
        permission = os.environ.get(PLACE)
        require(permission in (None, '0', '1') and (not placement or permission == '1'),
                'explicit route batch placement permission required')
        tokens = tuple(os.environ.get(key, '').encode('utf-8') for key in (native.TOKEN, OPS_TOKEN, MAP_TOKEN))
        require(all(32 <= len(token) <= 256 and b'\0' not in token for token in tokens),
                'invalid route batch credentials')
        return cls(native.endpoint(os.environ.get(ENDPOINT, '127.0.0.1:5000')), *tokens, permission == '1')

    def guard(self, operation: str = 'QueryPlacement') -> None:
        require(operation in native.METHODS, 'unknown route batch operation')
        require(Authority.load(operation in ('PreparePlacement', 'CommitPlacement')) == self,
                'route batch authority changed')


class Budget:
    """One shared foreground allowance for both source families and all storage."""
    def __init__(self, timeout_ms: int = 10000):
        self.deadline = time.monotonic() + integer(timeout_ms, 1, 60000) / 1000
        self.calls, self.network_bytes = 560, 40 * 1024 * 1024
        self.disk_bytes, self.work_steps = 1024 * 1024 * 1024, 20_000_000

    def remaining(self):
        left = self.deadline - time.monotonic()
        require(left > 0, 'route batch foreground deadline exhausted')
        return left

    def work(self):
        self.remaining()
        require(self.work_steps > 0, 'route batch work allowance exhausted')
        self.work_steps -= 1

    def reserve(self, kind, count):
        require(kind in ('calls', 'network_bytes', 'disk_bytes'), 'unknown route budget class')
        integer(count, 0, 1024 * 1024 * 1024)
        self.remaining()
        require(count <= getattr(self, kind), 'route batch aggregate allowance exhausted')
        setattr(self, kind, getattr(self, kind) - count)

    def reserve_call(self):
        self.reserve('calls', 1)

    def reserve_bytes(self, count):
        self.reserve('network_bytes', count)


def selection(step):
    return Selection(KINDS.index(step.kind), step.item, *step.target)


def packet(operation: str, result: dict | None, error=False) -> dict:
    value = {'schema': 'dfmcp.route-furniture-batch-result/1', 'ok': not error,
        'result': result, 'runtime_admitted': False,
        **({'error': {'code': 'ROUTE_BATCH_REFUSED', 'detail':
            'Original custody, source, authority, confirmation or budget refused. Recover the original batch and keys; never retry placement.'}} if error else {}),
        'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'route_furniture_batch.' + operation,
            'phase': 'reconcile' if error else 'propose' if operation in ('review', 'review-run') else 'inspect',
            'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'indeterminate' if error else 'stale', 'basis': None, 'gap': None},
            'profile': 'tactical', 'briefing': {'runtime_admitted': False,
                'atomic_route_placement_proven': False, 'construction_completion_proven': False},
            'active_work': [] if result is None else result.get('steps', []),
            'changes': [], 'attention': [], 'affordances': [], 'recommendations': [],
            'coverage': {'original_route_custody': result is not None and result.get('inventory_verified', False),
                'global_controller_fence': False, 'continuous_route_preservation': False},
            'uncertainty': ['Placed receipts prove historical job registration, not completed construction.',
                'Fresh route gates do not lock the entire map atomically with native placement.',
                'Source and private-file custody are local; another controller or operator can act independently.'],
            'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False},
            'references': [] if result is None else [result.get('batch_id'), result.get('review_digest')]}}
    report = result.get('route_frontier') if type(result) is dict else None
    construction = report.get('construction_prefix') if type(report) is dict else None
    if type(construction) is dict:
        value['agent_turn']['briefing']['execution_frontier_status'] = report['status']
        value['agent_turn']['briefing']['predecessor_construction_status'] = construction['status']
        value['agent_turn']['coverage']['construction_prefix_checked'] = True
    if type(result) is dict and 'room_handoff_digest' in result:
        turn = value['agent_turn']
        turn['coverage']['complete_original_room_custody'] = result.get('inventory_verified', False)
        turn['coverage']['room_constraints_checked_this_call'] = bool(
            type(report) is dict and report.get('room_constraints_checked_this_call') is True)
        turn['briefing']['room_completion_proven'] = False
        turn['briefing']['atomic_room_placement_proven'] = False
        if type(report) is dict and 'room_terrain' in report:
            turn['briefing']['room_terrain_status'] = report['room_terrain']['status']
        turn['references'].append({'room_handoff_digest': result['room_handoff_digest'],
                                   'room_plan_digest': result['room_plan_digest']})
        turn['uncertainty'].append(
            'The whole original room is retained and checked at placement gates. '
            'These paused samples are not an atomic map lock, room assignment or completion proof.')
    return value


def encoded(operation, result, error=False):
    raw = canonical(packet(operation, result, error)) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete route batch response exceeds budget')
    return raw


def reserve(state, report=None):
    value = dict(state)
    if report is not None:
        value['route_frontier'] = report
    # Native outcome <=64 KiB; all possible remaining per-step/index metadata
    # and Agent Turn copies reserve a further 32 KiB BEFORE child creation.
    require(len(encoded('advance', value)) + 96 * 1024 <= MAX_OUTPUT,
            'complete native outcome cannot fit route batch output')


def next_step(batch):
    state = batch.audit()
    require(state['advance_allowed'], 'route batch stopped, complete or pending original-key recovery')
    return next(s for s in batch.plan.ordered if s.name == state['next_step']), state


def confirmation(batch, plan: Plan, inventory: str, report: dict):
    return hashlib.sha256(b'dfmcp-route-batch-confirmation/1\0' + canonical({
        'batch': batch.id, 'review': batch.review.digest, 'inventory': inventory,
        'native_plan': plan.digest.hex(), 'route_evidence': report['evidence_digest']})).hexdigest()


def acquire_frontier(batch, before, authority, budget):
    """One unchanged read bracket, then close and check all remaining selections."""
    import route_furniture_inventory as inventory
    import route_furniture_allocation as routes
    from construction_receipt import decode_operations
    from construction_frontier import gate_frontier

    class ReadAuthority(inventory.Authority):
        def guard(self):
            authority.guard()

    require(isinstance(authority, Authority) and authority.address == batch.address,
            'route batch endpoint differs from original custody')
    require(budget.work_steps > ANALYSIS_RESERVATION, 'complete route analysis not admitted')
    authority.guard()
    batch.check()
    read = ReadAuthority(authority.address, authority.operations_token, authority.map_token)
    region = inventory.Region(batch.source.origin, batch.source.size)
    with inventory.Client(read, region, budget) as client:
        require((client.manifests['map'].generation, client.manifests['operations'].generation,
                 *client.manifests['map'].software) ==
                (batch.source.map_generation, batch.source.inventory_generation,
                 batch.source.df_version, batch.source.dfhack_version), 'original route incarnations changed')
        map_manifest, raw, operations_manifest, operations_raw = client.capture_once()
    require(client.closed, 'route connection must close before projection')
    terrain = routes.decode_terrain(raw, budget.work)
    observed = decode_operations(operations_raw, budget.work)
    require((observed.folder, observed.site, observed.tick, observed.paused) ==
            (batch.request.folder, batch.request.site, terrain.tick, True)
            and terrain.paused, 'route sources disagree on the original paused fortress')
    candidates, _ = inventory.candidates_from(observed, batch.request, budget.work)
    source = Source(batch.source.endpoint, map_manifest.df_version, map_manifest.dfhack_version,
        map_manifest.generation, operations_manifest.generation, terrain.digest, observed.digest,
        observed.tick, terrain.dimensions, terrain.origin, terrain.size, observed.horizons)
    prefix = batch.placed_prefix()
    require(budget.work_steps > ANALYSIS_RESERVATION, 'route projection consumed reserved analysis work')
    budget.work_steps -= ANALYSIS_RESERVATION
    def guard():
        budget.remaining()
        authority.guard()
    result = frontier.assess(batch.review, batch.id, prefix, before, candidates,
        {item.id: observed.items[item.id].native_type for item in candidates}, raw, source,
        guard, maximum_work=ANALYSIS_RESERVATION)
    # Placed is job registration, not finished hauling/construction. Preserve
    # the exact prefix and require current quiescence before another delivery.
    # This consumes the remainder of the SAME reserved analysis allowance.
    result = gate_frontier(result, prefix, operations_raw, guard,
                           maximum_work=ANALYSIS_RESERVATION)
    if batch.room_handoff is not None:
        from room_route_frontier import gate_frontier as gate_room
        # Reuse the exact raw map and unconsumed structural allowance. No
        # additional native read and no weakening of route/construction gates.
        result = gate_room(batch.room_handoff, result, raw, guard,
                           maximum_work=ANALYSIS_RESERVATION)
    batch.check()
    guard()
    return result


def initialize(path, review, authority, budget):
    require(type(authority) is Authority, 'closed route initialization authority required')
    origin = review
    if type(origin) is not Review:
        from room_route_handoff import RoomRouteHandoff
        require(type(origin) is RoomRouteHandoff, 'closed route initialization origin required')
        origin = RoomRouteHandoff.decode(origin.raw, origin.digest, budget.work)
        _, review = origin.unpack(budget.work)
    _, _, source, plan = review.unpack(budget.work)
    require(native.endpoint(source.endpoint) == authority.address, 'route endpoint differs before native contact')
    authority.guard()
    with native.Client(authority, budget, selection(plan.ordered[0])) as client:
        reply = client.observe()
    require(not reply.unresolved and reply.capture.eligible, 'initial placement source is unavailable')
    authority.guard()
    with Batch(path, None, budget, writable=True, initial=(origin, reply.manifest, reply.capture)) as batch:
        state = batch.audit()
        reserve(state)
        encoded('init', state)
        authority.guard()
        batch.check()
        return state


def validate_room_frontier(batch, report):
    if batch.room_handoff is None:
        return
    from room_route_frontier import POLICY, DOMAIN, MAX_OUTPUT as MAX_ROOM_FRONTIER
    batch.budget.remaining()
    room, review = batch.room_handoff.unpack(batch.budget.work)
    require(type(report) is dict and report.get('policy') == POLICY
            and report.get('room_handoff_digest') == batch.room_handoff.digest
            and report.get('room_plan_digest') == room.digest
            and report.get('review_digest') == review.digest
            and report.get('room_constraints_checked_this_call') is True,
            'room-backed placement requires its complete original room gate')
    require(len(canonical(report)) <= MAX_ROOM_FRONTIER, 'room execution report exceeds bound')
    payload = {key: value for key, value in report.items()
               if key not in ('work_units', 'maximum_work', 'evidence_digest')}
    require(report.get('evidence_digest') == hashlib.sha256(DOMAIN + canonical(payload)).hexdigest(),
            'room execution report changed after checking')
    if report.get('status') == 'ready':
        require(report.get('execution_frontier_status') == 'ready'
                and type(report.get('room_terrain')) is dict
                and report['room_terrain'].get('status') == 'matched'
                and report['room_terrain'].get('all_required_shapes_at_sample') is True,
                'room geometry or predecessor evidence is not ready')
    batch.budget.remaining()


def review_step(batch, authority, acquire=None):
    acquire = acquire_frontier if acquire is None else acquire
    step, state = next_step(batch)
    require(authority.address == batch.address, 'route endpoint changed')
    authority.guard()
    with native.Client(authority, batch.budget, selection(step)) as client:
        reply = client.observe()
    batch.bind(reply.capture, reply.manifest)
    require(not reply.unresolved and reply.retained_records < 256, 'native uncertainty or retention blocks review')
    plan = Plan(batch.key(step), reply.capture)
    report = acquire(batch, plan.before, authority, batch.budget)
    validate_room_frontier(batch, report)
    require(report['batch_id'] == batch.id and report['native_before_digest'] == plan.before.witness.hex(),
            'route review result bound to another capture')
    reserve(state, report)
    state.update(route_frontier=report, expected_plan=plan.digest.hex() if report['status'] == 'ready' else None,
                 confirm_review=confirmation(batch, plan, state['inventory_digest'], report) if report['status'] == 'ready' else None,
                 route_constraints_checked_this_call=True,
                 advance_allowed=report['status'] == 'ready')
    if batch.room_handoff is not None:
        state['room_constraints_checked_this_call'] = True
    encoded('review', state)
    authority.guard()
    batch.check()
    return state


def advance(batch, expected_plan, confirmed, authority, acquire=None):
    acquire = acquire_frontier if acquire is None else acquire
    digest_text(expected_plan)
    digest_text(confirmed)
    require(batch.writable and authority.address == batch.address, 'route placement lacks original writable custody')
    authority.guard('CommitPlacement')
    step, state = next_step(batch)
    reserve(state)
    prior = {name: j.raw for name, j in batch.effects.journals.items()}
    first = None
    def guard(plan, known, stage):
        nonlocal first
        authority.guard('CommitPlacement')
        batch.bind(plan.before, known.manifest)
        current = batch.audit()
        require(not batch.stopped and plan.key == batch.key(step)
                and plan.before.selection == selection(step) and not known.unresolved
                and known.retained_records < 256, 'route step or native fence changed')
        require(all(batch.effects.journals[name].raw == raw for name, raw in prior.items()),
                'predecessor evidence changed during route placement')
        if stage == 'before_intent':
            require(current['inventory_digest'] == state['inventory_digest'] and current['advance_allowed'],
                    'route batch changed since initial selection')
            first = acquire(batch, plan.before, authority, batch.budget)
            validate_room_frontier(batch, first)
            require(first['status'] == 'ready' and confirmed == confirmation(batch, plan, state['inventory_digest'], first),
                    'stale, unready or changed route confirmation')
            reserve(current, first)
        else:
            require(first is not None and current['pending_step'] == step.name, 'original route child is not pending')
            if stage == 'before_prepare':
                batch.register(step, first['evidence_digest'])
            elif stage == 'before_commit':
                # Dispatch intent has ALREADY been synced by the original client.
                # A failure here cannot roll it back or create a retry permit.
                fresh = acquire(batch, plan.before, authority, batch.budget)
                validate_room_frontier(batch, fresh)
                require(fresh['status'] == 'ready' and fresh['evidence_digest'] == first['evidence_digest'],
                        'route constraints or complete evidence changed before commit')
                with native.Client(authority, batch.budget, selection(step)) as check:
                    last = check.observe()
                require(last.manifest == batch.native and not last.unresolved
                        and last.capture.raw == plan.before.raw, 'native placement changed after route check')
            else:
                raise ValueError('unknown placement guard stage')
            require(any(row['slot'] == step.name and row['route_evidence_digest'] == first['evidence_digest']
                        for row in batch.entries), 'route proof lacks durable original child association')
        authority.guard('CommitPlacement')
        batch.check()
    native_result, _ = placement.start(batch.effects, authority, selection(step), batch.key(step), expected_plan, guard)
    state = batch.audit()
    state.update(placement=native_result, advanced_step=step.name, route_frontier=first,
                 route_constraints_checked_this_call=True)
    if batch.room_handoff is not None:
        state['room_constraints_checked_this_call'] = True
    encoded('advance', state)
    authority.guard()
    batch.check()
    return state


def recover(batch, step_name, authority=None, *, cancel=False):
    require(batch.writable, 'route recovery requires writable journal custody')
    step = next((s for s in batch.plan.steps if s.name == step_name), None)
    require(step is not None, 'recovery step outside original route plan')
    journal = batch.effects.get(batch.key(step))
    batch.register(step, None)  # Crash-before-registration recovery, NEVER a route permit.
    result = placement.recover(journal, authority, cancel)
    state = batch.audit()
    state.update(recovered_step=step_name, recovery=result)
    encoded('cancel' if cancel else 'query', state)
    if authority is not None:
        authority.guard('QueryPlacement')
    batch.check()
    return state


def read_review(path, expected, budget, *, room_backed=False):
    digest_text(expected)
    require(type(room_backed) is bool, 'explicit room input selection required')
    maximum = 49152
    if room_backed:
        from room_route_handoff import MAX_BYTES, RoomRouteHandoff
        maximum = MAX_BYTES
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and 0 < before.st_size <= maximum, 'invalid route review input')
        budget.reserve('disk_bytes', before.st_size + 1)
        raw = bytearray()
        while len(raw) <= before.st_size:
            budget.work()
            data = os.read(fd, min(4096, before.st_size + 1 - len(raw)))
            if not data:
                break
            raw += data
        import build_placement_store as storage
        require(len(raw) == before.st_size and storage.stamp(before) == storage.stamp(os.fstat(fd))
                == storage.stamp(os.stat(path, follow_symlinks=False)), 'route review input changed')
        if room_backed:
            return RoomRouteHandoff.decode(bytes(raw), expected, budget.work)
        return Review.decode(bytes(raw), expected, budget.work)
    finally:
        os.close(fd)


def publish(raw):
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError, KeyboardInterrupt):
        return 2
    return 0


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid route batch arguments')


def main(argv=None):
    operation, batch, authority = 'unknown', None, None
    output_attempted = False
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('init', 'inspect', 'review', 'advance', 'query', 'cancel', 'stop', 'review-run', 'run'))
        parser.add_argument('--directory', required=True)
        parser.add_argument('--batch-id')
        parser.add_argument('--review-file')
        parser.add_argument('--expected-review')
        parser.add_argument('--room-handoff-file')
        parser.add_argument('--expected-room-handoff')
        parser.add_argument('--expected-plan')
        parser.add_argument('--confirm-review')
        parser.add_argument('--max-steps', type=int)
        parser.add_argument('--confirm-run')
        parser.add_argument('--step')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        operation = args.operation
        require((args.batch_id is None) == (operation == 'init'), 'original batch ID is required after initialization')
        route_pair = (args.review_file, args.expected_review)
        room_pair = (args.room_handoff_file, args.expected_room_handoff)
        require(all(v is None for v in route_pair) or all(v is not None for v in route_pair),
                'complete original review and digest required')
        require(all(v is None for v in room_pair) or all(v is not None for v in room_pair),
                'complete original room handoff and digest required')
        origins = int(args.review_file is not None) + int(args.room_handoff_file is not None)
        require(origins == int(operation == 'init'),
                'initialize with exactly one complete origin; later operations cannot replace it')
        require(all((v is not None) == (operation == 'advance') for v in (args.expected_plan, args.confirm_review)),
                'advance requires exact native plan and route confirmation')
        require((args.step is not None) == (operation in ('query', 'cancel')), 'recovery alone selects an original child')
        running = operation in ('review-run', 'run')
        require((args.max_steps is not None) == running, 'only run/review-run require an explicit step cap')
        require((args.confirm_run is not None) == (operation == 'run'), 'run requires its separate exact confirmation')
        if running:
            import route_furniture_run as runner
            limits = runner.Limits(args.max_steps, args.timeout_ms)
            if args.confirm_run is not None:
                digest_text(args.confirm_run)
            budget = runner.make_budget(limits, _steps=sys.modules[__name__])
        else:
            budget = Budget(args.timeout_ms)
        if operation == 'init':
            saved = (read_review(args.room_handoff_file, args.expected_room_handoff, budget, room_backed=True)
                     if args.room_handoff_file is not None else
                     read_review(args.review_file, args.expected_review, budget))
            authority = Authority.load()
            state = initialize(args.directory, saved, authority, budget)
            # Reacquire and hold the just-published original through stdout.
            batch = Batch(args.directory, state['batch_id'], budget)
            state = batch.audit()
        else:
            batch = Batch(args.directory, args.batch_id, budget, writable=operation in ('advance', 'query', 'cancel', 'stop', 'run'))
            if running:
                authority = Authority.load(operation == 'run')
                state = (runner.review_run(batch, authority, limits, _steps=sys.modules[__name__])
                         if operation == 'review-run' else
                         runner.run(batch, authority, limits, args.confirm_run, _steps=sys.modules[__name__]))
            elif operation in ('review', 'advance'):
                authority = Authority.load(operation == 'advance')
                state = (review_step(batch, authority) if operation == 'review' else
                         advance(batch, args.expected_plan, args.confirm_review, authority))
            elif operation in ('query', 'cancel'):
                # Recovery deliberately needs only the unchanged native read
                # environment; missing map/inventory credentials cannot force retry.
                step = next((s for s in batch.plan.steps if s.name == args.step), None)
                require(step is not None, 'unknown original recovery step')
                journal = batch.effects.get(batch.key(step))
                authority = native.Authority.load() if placement.recovery_requires_native(journal, operation == 'cancel') else None
                state = recover(batch, args.step, authority, cancel=operation == 'cancel')
            else:
                state = batch.stop() if operation == 'stop' else batch.audit()
        faulted = operation == 'run' and state['run']['faulted']
        raw = encoded(operation, state, error=faulted)
        if authority is not None:
            authority.guard('QueryPlacement')
        batch.check()
        budget.remaining()
        output_attempted = True
        published = publish(raw)  # Hold custody through the single output attempt.
        return 2 if faulted else published
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if not output_attempted:
            publish(encoded(operation, None, True))
        return 2
    finally:
        if batch is not None:
            batch.close()


if __name__ == '__main__':
    raise SystemExit(main())