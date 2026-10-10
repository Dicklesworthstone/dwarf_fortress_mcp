#!/usr/bin/env python3
"""Monitor original room terrain and installed furnishings as one durable goal.

Each explicit sample checks every original placement receipt around equal paused
map endpoints and one released operations capture. The complete original batch,
room intention and fixed game-time policy survive restart. No game effect or
time-control method is bound, and the monitor never repeats a placement.
Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5; WP-05/07/10.
"""
from __future__ import annotations

import argparse
import sys

from build_placement_wire import Rejected, canonical, require
from construction_plan import Goal as Condition
from furniture_batch import Batch
from furniture_completion import Origin
from room_readiness import POLICY, Progress
from room_readiness_rpc import Authority, Budget, acquire
from room_readiness_store import Goal, Journal, State, MAX_FILE, MAX_FRAMES
import track_furniture_batch as furnishing
import construction_wait as foreground

MAX_OUTPUT = 256 * 1024


def packet(operation: str, state: State | None, *, source_verified: bool = False,
           native_contacted: bool = False, storage_acknowledged: bool = False,
           verified: bool = True, error: bool = False) -> dict:
    """Keep the entire original project visible beside one joint stability window."""
    value = furnishing.packet(operation, state, source_verified=source_verified,
        native_contacted=native_contacted, storage_acknowledged=storage_acknowledged,
        verified=verified, error=error)
    value['schema'] = 'dfmcp.room-readiness-monitor-result/1'
    result, turn = value['result'], value['agent_turn']
    progress = None if state is None else state.progress
    original = (source_verified and verified and not error and state is not None)
    satisfied = bool(original and progress.phase == 'satisfied')
    observed = bool(verified and progress is not None and progress.map_witness is not None)
    result.update(condition_policy=POLICY,
        room_readiness_sampled_condition=satisfied,
        room_completion_proven=False, terrain_completion_proven=False,
        room_assignments_observed=False, atomic_cross_profile_snapshot_proven=False,
        continuous_wall_preservation_proven=False, production_admitted=False)
    result.setdefault('requested_room_plan', None)
    result.setdefault('room_origin', None)
    if result['progress'] is not None:
        # Cancel can inspect a previously satisfied terminal journal without
        # reopening the original files. Historical evidence remains visible,
        # but the combined claim still requires original custody in this call.
        result['progress']['historical_joint_condition_satisfied'] = progress.phase == 'satisfied'
        result['progress']['room_readiness_sampled_condition'] = satisfied
        # An unresolved read preserves the previous sample as history, but
        # cannot carry its stability credit into the next acquisition.
        result['progress']['effective_streak'] = 0 if progress.reading else progress.streak
    if state is not None:
        result['identity']['selection'] = 'every_original_room_requirement_and_furnishing'
        result['identity']['room_plan_digest'] = state.goal.origin.room_handoff.room_plan.digest
        result['identity']['joint_condition_digest'] = state.goal.readiness_goal.digest
    turn['operation'] = 'room_readiness.' + operation
    turn['briefing'].update(
        construction_scope='original_room_terrain_and_all_original_furnishings_joint_sample_only',
        room_readiness_sampled_condition=satisfied, room_completion_proven=False,
        terrain_completion_proven=False)
    turn['coverage'].update(
        room_terrain_observed=observed,
        original_room_custody_verified=bool(original),
        terrain_and_furnishings_share_stability_window=True,
        atomic_cross_profile_snapshot_proven=False,
        continuous_wall_preservation_proven=False,
        room_assignments_observed=False)
    turn['budget']['output_bytes_limit'] = MAX_OUTPUT
    for work in turn['active_work']:
        work['kind'] = 'room_readiness_monitor'
    if observed:
        turn['references'].append({
            'kind': 'joint_room_readiness_sample', 'profile': POLICY,
            'sha256': progress.sample_digest, 'game_tick': progress.last_tick,
            'map_observation_witness': progress.map_witness,
            'historical': True, 'canonical_world_anchor': False,
        })
    turn['uncertainty'] = [
        'Every original room floor, required wall and receipt-linked furnishing must satisfy one joint sampled condition; separately timed successes cannot complete it.',
        'Original receipt queries bracket equal paused map endpoints surrounding a released complete operations capture. They do not prove an atomic cross-profile snapshot.',
        'Satisfaction records a historical sampled stability window, not current usability or continuous wall preservation. Map and operations generations remain separate namespaces.',
        'Room assignments, structural safety, pathfinding and construction causality are not established by this monitor.',
        'Combined readiness requires verified original batch custody in this call. Cancellation verifies only the monitor journal and never discharges or retries a placement effect.',
    ]
    if error:
        value['error'] = {
            'code': 'ROOM_READINESS_MONITOR_REFUSED',
            'detail': 'Original room batch, complete evidence, source, authority, budget or monitor custody refused. Preserve the monitor and original placement history; do not retry placement.',
        }
    return value


def output(value: dict) -> bytes:
    raw = canonical(value) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete original room readiness result exceeds output allowance')
    return raw


def publish(raw: bytes) -> int:
    """A partial stdout write never causes another object or native acquisition."""
    try:
        text = raw.decode('ascii')
        if sys.stdout.write(text) != len(text):
            return 2
        sys.stdout.flush()
    except (OSError, ValueError, TypeError, KeyboardInterrupt):
        return 2
    return 0


def reserve(operation: str, state: State, *, source_verified: bool = False,
            native_contacted: bool = False, storage_acknowledged: bool = False,
            wait_limits: foreground.Limits | None = None) -> bytes:
    value = packet(operation, state, source_verified=source_verified,
                   native_contacted=native_contacted, storage_acknowledged=storage_acknowledged)
    value['result']['journal'] = {'frames': MAX_FRAMES, 'bytes': MAX_FILE, 'head': 'f' * 64}
    if wait_limits is not None:
        value['result']['wait'] = foreground.reserve_view(wait_limits)
    return output(value)


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise Rejected('invalid room-readiness arguments')


def main(argv: list[str] | None = None) -> int:
    operation, owner, batch, last_state = 'unknown', None, None, None
    native_contacted, authority = False, None
    wait_result = None
    try:
        parser = Parser(description=__doc__)
        parser.add_argument('operation', choices=('start', 'sample', 'wait', 'inspect', 'cancel'))
        parser.add_argument('--journal', required=True)
        parser.add_argument('--batch')
        parser.add_argument('--batch-id')
        parser.add_argument('--deadline-tick', type=int)
        parser.add_argument('--interval-ticks', type=int)
        parser.add_argument('--stable-samples', type=int)
        parser.add_argument('--stable-span-ticks', type=int)
        parser.add_argument('--max-gap-ticks', type=int)
        parser.add_argument('--max-observations', type=int)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        foreground.add_arguments(parser)
        args = parser.parse_args(argv)
        operation = args.operation
        wait_limits = foreground.parse_limits(args)
        budget = Budget(args.timeout_ms)
        initial = (args.batch, args.batch_id, args.deadline_tick)
        choices = (args.interval_ticks, args.stable_samples, args.stable_span_ticks,
                   args.max_gap_ticks, args.max_observations)
        require(all(value is not None for value in initial) if operation == 'start'
                else all(value is None for value in initial + choices),
                'start requires original room batch and fixed policy; reopening cannot replace them')
        if operation == 'start':
            batch = Batch(args.batch, budget)
            origin = Origin.from_batch(batch, args.batch_id)
            furnishing.separate_journal(args.journal, origin.batch_path)
            condition = Condition(origin.receipts, args.deadline_tick,
                **{name: value for name, value in zip(
                    ('interval', 'stable_samples', 'stable_span', 'max_gap', 'max_observations'), choices)
                    if value is not None})
            goal = Goal(origin, condition, guard=budget.work)
            authority = Authority.load()
            require(authority.address == origin.address,
                    'room query endpoint differs from original furnishing batch')
            reserve(operation, State(goal, origin.address, Progress(goal.digest)), source_verified=True)
            origin.verify_batch(batch)
            authority.guard()
            owner = Journal(args.journal, budget, writable=True, create=(goal, origin.address))
        else:
            owner = Journal(args.journal, budget, writable=operation in ('sample', 'wait', 'cancel'))
            last_state = owner.state
            if operation != 'cancel':
                furnishing.separate_journal(args.journal, owner.state.goal.origin.batch_path)
                batch = Batch(owner.state.goal.origin.batch_path, budget)
        if batch is not None:
            owner.bind_batch(batch)

        def source_guard() -> None:
            require(batch is not None, 'original room batch custody is unavailable')
            owner.state.goal.origin.verify_batch(batch)
            budget.remaining()

        last_state = owner.state
        stored = operation == 'start'
        if operation != 'cancel':
            source_guard()
        if operation in ('start', 'sample') and not owner.state.progress.terminal:
            if operation != 'start':
                authority = Authority.load()
            require(authority.address == owner.state.address,
                    'query endpoint differs from original room readiness goal')
            reserve(operation, owner.state, source_verified=True,
                    native_contacted=True, storage_acknowledged=True)
            authority.guard()
            source_guard()
            owner.start_read()
            last_state = owner.state
            authority.guard()
            budget.remaining()
            native_contacted = True
            sample = acquire(authority, owner.state.goal.readiness_goal, budget)

            def render(candidate: State) -> None:
                reserve(operation, candidate, source_verified=True,
                        native_contacted=True, storage_acknowledged=True)
                authority.guard()
                source_guard()

            owner.accept(sample, render)
            stored = True
        elif operation == 'wait':
            authority = None if owner.state.progress.terminal else Authority.load()

            def acquire_wait(current_authority, original_goal, current_budget):
                nonlocal native_contacted
                native_contacted = True
                return acquire(current_authority, original_goal.readiness_goal, current_budget)

            wait_result = foreground.run(owner, authority, acquire_wait,
                lambda candidate: reserve(operation, candidate, source_verified=True,
                    native_contacted=False, storage_acknowledged=False, wait_limits=wait_limits),
                wait_limits, source_guard=source_guard,
                # Two map bindings, one handshake and both map endpoints are
                # required in addition to the complete receipt-monitor sample.
                additional_rpc_calls=5)
            stored = wait_result.samples > 0
        elif operation == 'cancel':
            stored = owner.cancel(lambda candidate: reserve(operation, candidate,
                                                            storage_acknowledged=True))
        owner.check()
        if operation != 'cancel':
            source_guard()
        last_state = owner.state
        value = packet(operation, last_state, source_verified=operation != 'cancel',
                       native_contacted=native_contacted, storage_acknowledged=stored)
        value['result']['journal'] = {
            'frames': last_state.frames, 'bytes': owner.length, 'head': last_state.tail.hex()}
        if wait_result is not None:
            value['result']['wait'] = wait_result.view()
        raw = output(value)
        # Final rendering cannot outlive source custody or the authority that
        # acquired the newly persisted sample, including a terminal sample.
        if authority is not None:
            authority.guard()
        owner.check()
        if operation != 'cancel':
            source_guard()
        budget.remaining()
        owner.close()
        owner = None
        if batch is not None:
            batch.close()
            batch = None
        budget.remaining()
        if authority is not None:
            authority.guard()
        return publish(raw)
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        if owner is not None:
            last_state = owner.state
        value = packet(operation, last_state, native_contacted=native_contacted,
                       verified=False, error=True)
        publish(output(value))
        return 2
    finally:
        if owner is not None:
            owner.close()
        if batch is not None:
            batch.close()


if __name__ == '__main__':
    raise SystemExit(main())
