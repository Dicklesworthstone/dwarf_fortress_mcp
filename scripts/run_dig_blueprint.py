#!/usr/bin/env python3
"""Review and execute a bounded original dig prefix in one foreground call.

This is an explicit development command, not an MCP or admitted runner. Each
new step uses the existing write-ahead coordinator and a fresh ONE-SHOT client.
Failure stops the call; query/cancel/recovery are never automatic. Original
batch and effect locks, keys, native codecs and receipt checks remain owners.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import sys

import dig_blueprint_client as bp
import dig_blueprint_prefix as policy

SCHEMA = 'dfmcp.dig-blueprint-prefix-result/1'
RPC_PER_STEP = 11  # Six bindings, handshake, two reads, prepare, one commit.
WIRE_PER_STEP = 4 * 1024 * 1024  # Includes each frame's bounded notifications.
ROW_RESERVATION = 2048  # New inventory row plus compact per-step outcome.
FINAL_RESERVATION = 8192


class Budget(bp.Budget):
    """One initial allowance, never renewed at a step or connection boundary."""
    def __init__(self, timeout_ms: int, steps: int):
        policy.integer(steps, 1, policy.MAX_STEPS)
        super().__init__(timeout_ms)
        self.step_allowance = steps
        self.calls_left = steps * RPC_PER_STEP
        self.network_left = steps * WIRE_PER_STEP
        self.connections_started = 0

    def admit_connection(self) -> None:
        self.check()
        policy.require(self.connections_started < self.step_allowance
                       and self.calls_left >= RPC_PER_STEP
                       and self.network_left >= WIRE_PER_STEP,
                       'insufficient shared allowance for another complete original step')
        self.connections_started += 1


def inventory(batch) -> dict:
    with bp.s.open_store(bp.d, batch.effects, budget=batch.budget.check) as store:
        return batch.inventory(store)


def selection(batch, state: dict, count: int) -> policy.Prefix:
    selected = policy.Prefix.create(state, batch.layout.regions(), count,
        allow_hidden_neighbors=batch.value['allow_hidden_neighbors'],
        checkpoint_policy=batch.value['checkpoint_policy'], guard=batch.budget.check)
    for step in selected.json()['steps']:
        policy.require(step['key'] == batch.step_key(step['step']), 'original native key generation changed')
    return selected


def packet(operation: str, batch, state: dict, selected: policy.Prefix,
           outcomes: list[dict], reason: str, budget: Budget) -> dict:
    complete = reason in ('prefix_completed', 'all_designations_verified')
    result = {'schema': SCHEMA, 'ok': operation == 'review' or complete,
        'operation': operation, 'review': selected.json(), 'batch': state,
        'blueprint': batch.layout.blueprint(),
        'run': {'stop_reason': reason, 'steps_considered': len(outcomes),
                'steps_recorded': sum(row['state'] != 'not_started' for row in outcomes), 'outcomes': outcomes,
                'connections_started': budget.connections_started,
                'rpc_calls_used': budget.step_allowance * RPC_PER_STEP - budget.calls_left,
                'native_bytes_used': budget.step_allowance * WIRE_PER_STEP - budget.network_left,
                'commit_attempts_acknowledged': sum(row['commit_attempted'] for row in outcomes),
                'reviewed_prefix_designations_verified': complete},
        'atomic': False, 'automatic_recovery_or_retry': False,
        'retry_commit_permitted': False, 'excavation_completion_proven': False,
        'current_terrain_proven': False, 'production_admitted': False}
    result['agent_turn'] = {
        'schema': 'dfmcp.agent_turn/1', 'operation': 'dig_blueprint.' + operation + '_prefix',
        'phase': 'propose' if operation == 'review' else 'inspect',
        'session_id': None, 'request_id': None, 'anchor': None,
        'continuity': {'status': 'bootstrap', 'basis': None, 'gap': None, 'reset_reason': None},
        'briefing': {'batch_id': batch.id, 'stop_reason': reason,
                     'designated_steps': state['designated_steps'],
                     'total_steps': state['total_steps'], 'runtime_admitted': False},
        'changes': [], 'attention': [], 'active_work': [], 'affordances': [],
        'recommendations': [],
        'uncertainty': [
            'The prefix is not atomic; earlier effects survive a later refusal or interruption.',
            'Native receipts prove historical designation readback, not finished excavation or current safety.',
            'Uncertain work requires explicit original-key query or cancellation; this command never retries it.'],
        'coverage': {'domain': 'original_dig_batch_prefix', 'original_request_retained': True,
                     'canonical_world_observation': False},
        'budget': {'output_bytes_limit': bp.ROOM_MAX_OUTPUT if batch.room_handoff else bp.MAX_OUTPUT,
                   'token_count_measured': False},
        'references': [{'kind': 'original_dig_batch', 'digest': batch.id, 'canonical_anchor': False}]}
    return result


def reserve(batch, state: dict, selected: policy.Prefix, budget: Budget) -> None:
    """Bound the entire possible response before any new native contact/effect."""
    raw = bp.serialize_result(packet('run', batch, state, selected, [], 'prefix_completed', budget), budget)
    maximum = bp.ROOM_MAX_OUTPUT if batch.room_handoff else bp.MAX_OUTPUT
    policy.require(len(raw) + selected.json()['step_count'] * ROW_RESERVATION + FINAL_RESERVATION <= maximum,
                   'complete original-prefix result cannot fit before dispatch')


def review(root: Path, steps: int, budget: Budget) -> dict:
    policy.require(budget.step_allowance == steps, 'review allowance differs from selected count')
    with bp.open_batch(root, budget) as batch:
        state = inventory(batch)
        selected = selection(batch, state, steps)
        reserve(batch, state, selected, budget)
        result = packet('review', batch, state, selected, [], 'reviewed', budget)
        bp.serialize_result(result, budget)
        policy.require(inventory(batch) == state, 'original batch changed during prefix review')
        return result


def execute_step(batch, before: dict, step: dict, budget: Budget, credentials,
                 client_factory=None) -> tuple[dict | None, dict]:
    """Use the existing coordinator without changing its one-shot commit rule."""
    index, region = step['step'], step['region']
    policy.require(before['next_step'] == index and batch.step_key(index) == step['key'],
                   'next original step differs from reviewed prefix')
    policy.require(inventory(batch) == before, 'original inventory changed before next step')
    budget.admit_connection()
    # A separate connection is planned for each NEW key. No failure reconnects,
    # and neither the native Client nor its commit_attempted flag is reset.
    factory = bp.RoomClient if client_factory is None else client_factory
    with factory(credentials, budget, batch.verify) as raw:
        client = bp.CheckedClient(batch, raw, credentials, region, before)
        native = client.observe(region)
        observed = bp.d.observation(native['raw'], region)
        blockers = bp.d.blockers(observed, batch.value['allow_hidden_neighbors'])
        if blockers:
            policy.require(inventory(batch) == before, 'original inventory changed during refused preflight')
            budget.check()
            return None, {'step': index, 'key': step['key'], 'state': 'not_started',
                          'commit_attempted': False, 'blockers': blockers,
                          'observation_witness': observed['witness']}
        witness = observed['witness']
        plan = bp.d.plan_for(region, batch.value['allow_hidden_neighbors'], bytes.fromhex(witness)).hex()

        def guard(store, owner):
            current = batch.inventory(store, owner)
            if owner is None:
                policy.require(current == before, 'original prefix changed before preparation')
            else:
                policy.require(len(current['records']) == index + 1
                    and current['records'][:index] == before['records']
                    and current['records'][index]['key'] == step['key']
                    and current['records'][index]['plan_digest'] == plan,
                    'original prefix changed across native effect boundary')
            budget.check()

        result = bp.s.start_designation(bp.d, client, batch.effects / batch.step_name(index),
            step['key'], region, batch.value['allow_hidden_neighbors'], witness, plan, guard=guard)
        after = inventory(batch)
        budget.check()
        policy.require(type(result['commit_attempted_this_call']) is bool,
                       'native result lost exact dispatch status')
        return after, {'step': index, 'key': step['key'], 'state': after['records'][-1]['state'],
                       'commit_attempted': result['commit_attempted_this_call'], 'blockers': [],
                       'observation_witness': witness}


def run(root: Path, batch_id: str, steps: int, confirm_run: str, budget: Budget,
        client_factory=None) -> dict:
    policy.identity(batch_id)
    policy.identity(confirm_run)
    policy.require(budget.step_allowance == steps, 'run allowance differs from confirmed count')
    with bp.open_batch(root, budget) as batch:
        state = inventory(batch)
        selected = selection(batch, state, steps)
        selected.confirm(batch_id, confirm_run)
        reserve(batch, state, selected, budget)
        credentials = bp.d.environment(True, batch.value['endpoint'])
        # Legacy and room batches both use the revocable outer authority fence.
        budget.bind_room(credentials, True)
        outcomes = []
        reason = 'prefix_completed'
        for step in selected.json()['steps']:
            budget.check()
            after, outcome = execute_step(batch, state, step, budget, credentials, client_factory)
            outcomes.append(outcome)
            if after is None:
                reason = 'preflight_blocked'
                break
            status = selected.successor(state, after, budget.check)
            state = after
            if status != 'designated':
                reason = 'indeterminate' if status == 'unknown' else 'native_refused'
                break
        if reason == 'prefix_completed' and state['phase'] == 'designations_verified':
            reason = 'all_designations_verified'
        policy.require(inventory(batch) == state, 'original inventory changed before run publication')
        result = packet('run', batch, state, selected, outcomes, reason, budget)
        bp.serialize_result(result, budget)
        batch.verify()
        budget.check()
        return result


def publish(raw: bytes) -> int:
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError, KeyboardInterrupt):
        return 2
    return 0


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError('invalid original-prefix arguments')


def refusal() -> dict:
    return {'schema': SCHEMA, 'ok': False, 'result': None,
        'effect_status': 'unknown', 'error': {'code': 'DIG_PREFIX_RUN_REFUSED',
            'detail': 'Original prefix, authority, custody, budget or native evidence refused. '
                      'Earlier steps may have effects. Inspect the original batch; never repeat uncertain keys.'},
        'automatic_recovery_or_retry': False, 'retry_commit_permitted': False,
        'excavation_completion_proven': False, 'production_admitted': False,
        'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'dig_blueprint.prefix',
            'phase': 'inspect', 'session_id': None, 'request_id': None, 'anchor': None,
            'continuity': {'status': 'bootstrap', 'basis': None, 'gap': None, 'reset_reason': None},
            'briefing': {'effect_status': 'unknown', 'runtime_admitted': False},
            'changes': [], 'attention': [], 'active_work': [], 'affordances': [], 'recommendations': [],
            'uncertainty': ['Earlier steps may have effects; original custody and current outcome were not established.',
                            'Retain the original batch and inspect/query its original keys before further action.'],
            'coverage': {'domain': 'original_dig_batch_prefix', 'original_request_verified': False},
            'budget': {'output_bytes_limit': bp.MAX_OUTPUT, 'token_count_measured': False},
            'references': []}}


def main(argv=None) -> int:
    try:
        parser = Parser(description=__doc__, allow_abbrev=False)
        parser.add_argument('operation', choices=('review', 'run'))
        parser.add_argument('--directory', type=Path, required=True)
        parser.add_argument('--steps', type=int, required=True)
        parser.add_argument('--batch-id')
        parser.add_argument('--confirm-run')
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms, args.steps)
        policy.require((args.batch_id is not None and args.confirm_run is not None) if args.operation == 'run'
                       else args.batch_id is None and args.confirm_run is None,
                       'run requires explicit complete prefix confirmation; review cannot execute')
        result = (review(args.directory, args.steps, budget) if args.operation == 'review'
                  else run(args.directory, args.batch_id, args.steps, args.confirm_run, budget))
        raw = bp.serialize_result(result, budget) + b'\n'
        budget.check()  # Includes authority after final serialization and owner close.
        code = 0 if result['ok'] else 2
    except (ValueError, OSError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        # Never hide uncertain native effects behind an automatic re-open/query.
        # The original batch remains the only recovery owner after any failure.
        raw = policy.canonical(refusal()) + b'\n'
        code = 2
    return code if publish(raw) == 0 else 2


if __name__ == '__main__':
    raise SystemExit(main())
