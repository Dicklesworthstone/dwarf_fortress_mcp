"""Confirm a bounded original furnishing prefix, never substitute or retry items.

The batch owner authenticates journals, receipts, source and optional original
room/terrain custody. This pure policy only narrows that owner's next steps.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; WP-05/WP-10.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import re
from typing import Callable

from furniture_plan import FurniturePlan, canonical, integer, progress, require

SCHEMA = 'dfmcp.furniture-prefix-review/1'
POLICY = 'original-furniture-prefix-fresh-native-plans/1'
MAX_STEPS = 8
MAX_REVIEW = 16384
MAX_INVENTORY = 192 * 1024
Guard = Callable[[], None]


def idle() -> None:
    pass


def identity(value: object) -> str:
    require(type(value) is str and re.fullmatch('[0-9a-f]{64}', value) is not None,
            'invalid furniture prefix identity')
    return value


def key(value: object) -> str:
    require(type(value) is str and re.fullmatch('[A-Za-z0-9_.-]{1,128}', value) is not None,
            'invalid original furniture key')
    return value


def fingerprint(value: object) -> str:
    return hashlib.sha256(b'dfmcp-furniture-prefix-inventory/1\0' + canonical(value)).hexdigest()


def inventory(value: dict, guard: Guard = idle) -> FurniturePlan:
    """Validate the complete owner presentation, not imported receipt authority."""
    guard()
    required = {'status', 'next_step', 'pending_step', 'plan', 'plan_digest', 'placed',
                'total', 'steps', 'atomic', 'construction_completion_proven', 'retry_permitted',
                'batch_id', 'stopped', 'inventory_verified', 'advance_allowed', 'source',
                'endpoint', 'native_contacted', 'inventory_digest'}
    optional = {'allocation', 'room_origin', 'room_plan', 'terrain_origin',
                'terrain_history_verified_this_call', 'terrain_history_required_for_new_work'}
    require(type(value) is dict and required <= set(value) <= required | optional,
            'complete original furniture inventory required')
    for field in ('batch_id', 'plan_digest', 'inventory_digest'):
        identity(value[field])
    for field in ('atomic', 'construction_completion_proven', 'retry_permitted'):
        require(value[field] is False, 'inventory grants unsupported completion or retry proof')
    require(value['inventory_verified'] is True, 'original inventory not verified by owner')
    for field in ('stopped', 'advance_allowed', 'native_contacted'):
        require(type(value[field]) is bool, 'invalid inventory Boolean')
    plan = FurniturePlan.from_json(value['plan'])
    require(canonical(plan.json()) == canonical(value['plan']) and plan.digest == value['plan_digest'],
            'original furniture plan changed or is not canonical')
    rows = value['steps']
    require(type(rows) is list and len(rows) == len(plan.ordered), 'incomplete original step inventory')
    states = {}
    for step, row in zip(plan.ordered, rows):
        guard()
        require(type(row) is dict and row.get('name') == step.name, 'original step order changed')
        phase = row.get('phase')
        if phase == 'not_started':
            require(set(row) == {'name', 'phase'}, 'unstarted furniture carries effect evidence')
        else:
            require(set(row) == {'name', 'phase', 'key', 'plan_digest', 'registered',
                                 'receipt_digest', 'insertion', 'dispatch_recorded'},
                    'incomplete original child evidence')
            key(row['key'])
            identity(row['plan_digest'])
            require(type(row['registered']) is bool and type(row['dispatch_recorded']) is bool,
                    'invalid original child custody flags')
            if row['receipt_digest'] is not None:
                identity(row['receipt_digest'])
            if phase == 'placed':
                require(row['registered'] and row['receipt_digest'] is not None
                        and type(row['insertion']) is dict,
                        'placed prefix lacks registered original receipt')
            states[step.name] = phase
    expected = progress(plan, states)
    for field in ('status', 'next_step', 'pending_step', 'placed', 'total'):
        require(canonical(value[field]) == canonical(expected[field]), 'original progress contradicts complete history')
    allowed = (expected['status'] == 'ready' and not value['stopped']
               and all(row.get('registered', True) for row in rows))
    if 'terrain_origin' in value:
        require('room_origin' in value and 'room_plan' in value
                and value.get('terrain_history_required_for_new_work') is True
                and type(value.get('terrain_history_verified_this_call')) is bool,
                'original terrain custody scope is incomplete')
        allowed &= value['terrain_history_verified_this_call']
    else:
        require(not {'terrain_history_required_for_new_work', 'terrain_history_verified_this_call'} & set(value),
                'legacy inventory cannot adopt terrain evidence')
    require(value['advance_allowed'] == allowed, 'original admission differs from recorded custody')
    require(len(canonical(value)) <= MAX_INVENTORY, 'complete furniture inventory exceeds bound')
    guard()
    return plan


def stable_inventory(value: dict) -> dict:
    # Native contact is a per-call disclosure, not a change to durable custody.
    return {k: v for k, v in value.items() if k != 'native_contacted'}


def same_inventory(before: dict, after: dict, guard: Guard = idle) -> None:
    inventory(before, guard)
    inventory(after, guard)
    require(canonical(stable_inventory(before)) == canonical(stable_inventory(after)),
            'original furnishing inventory changed between review and dispatch')


@dataclass(frozen=True)
class Prefix:
    _body: bytes

    @classmethod
    def create(cls, state: dict, original_keys: dict[str, str], count: int,
               guard: Guard = idle) -> Prefix:
        plan = inventory(state, guard)
        integer(count, 1, MAX_STEPS)
        require(state['advance_allowed'], 'original batch is stopped, finished or requires recovery')
        require(type(original_keys) is dict and set(original_keys) == {s.name for s in plan.steps},
                'complete original key inventory required')
        for value in original_keys.values():
            guard()
            key(value)
        require(len(set(original_keys.values())) == len(original_keys), 'original furniture keys alias')
        for row in state['steps']:
            if row['phase'] != 'not_started':
                require(row['key'] == original_keys[row['name']], 'retained child key differs from original')
        start = state['placed']
        require(start + count <= len(plan.ordered), 'prefix exceeds remaining original furnishings')
        selected = plan.ordered[start:start + count]
        require(selected[0].name == state['next_step'], 'original furnishing frontier changed')
        body = {'schema': SCHEMA, 'policy': POLICY, 'native_profile': 'furniture/1.19',
                'batch_id': state['batch_id'], 'plan_digest': plan.digest,
                'inventory_digest': state['inventory_digest'],
                'inventory_commitment': fingerprint(stable_inventory(state)),
                'original_step_count': len(plan.steps), 'start_index': start, 'step_count': count,
                'steps': [{**s.json(), 'key': original_keys[s.name]} for s in selected],
                'confirmation_scope': 'original_exact_items_targets_and_dependencies_with_fresh_native_plans',
                'atomic': False, 'mutation_authority_granted': False,
                'automatic_recovery_or_retry': False, 'construction_completion_proven': False}
        raw = canonical(body)
        require(len(raw) + 96 <= MAX_REVIEW, 'complete furniture prefix exceeds review bound')
        guard()
        return cls(raw)

    @property
    def seal(self) -> str:
        return hashlib.sha256(b'dfmcp-furniture-prefix-review/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        return {**json.loads(self._body), 'confirm_run': self.seal}

    def confirm(self, batch_id: str, seal: str) -> None:
        identity(batch_id)
        identity(seal)
        require(batch_id == self.json()['batch_id'] and seal == self.seal,
                'stale or altered original furniture prefix confirmation')

    def successor(self, before: dict, after: dict, guard: Guard = idle) -> str:
        inventory(before, guard)
        inventory(after, guard)
        value = self.json()
        index = before['placed']
        require(before['advance_allowed'] and value['start_index'] <= index
                < value['start_index'] + value['step_count'], 'execution escaped original furnishing prefix')
        require(before['batch_id'] == value['batch_id'] and before['plan_digest'] == value['plan_digest'],
                'prefix belongs to another original batch')
        if index == value['start_index']:
            require(fingerprint(stable_inventory(before)) == value['inventory_commitment'],
                    'first placement differs from confirmed inventory')
        dynamic = {'status', 'next_step', 'pending_step', 'placed', 'steps', 'advance_allowed',
                   'inventory_digest', 'native_contacted'}
        require(canonical({k: v for k, v in before.items() if k not in dynamic})
                == canonical({k: v for k, v in after.items() if k not in dynamic}),
                'original plan, source, room or terrain custody changed')
        require(before['inventory_digest'] != after['inventory_digest'], 'new child lacks changed durable inventory')
        for offset, (old, new) in enumerate(zip(before['steps'], after['steps'])):
            guard()
            if offset != index:
                require(canonical(old) == canonical(new), 'prefix skipped work or rewrote another original child')
                continue
            step = value['steps'][index - value['start_index']]
            require(old['phase'] == 'not_started' and new['phase'] != 'not_started'
                    and new['name'] == step['name'] and new['key'] == step['key'],
                    'next original furnishing was repeated or substituted')
        guard()
        return after['steps'][index]['phase']
