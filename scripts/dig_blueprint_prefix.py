"""An exact reviewed prefix of an existing dig batch; no native authority.

The effect shell must reconstruct inventory under the original batch/store locks.
A seal confirms fixed targets and fresh per-step revalidation, not unseen terrain
or permission to repeat an original key. Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import re
from typing import Callable

SCHEMA = 'dfmcp.dig-blueprint-prefix-review/1'
POLICY = 'original-prefix-with-fresh-native-revalidation/1'
CHECKPOINT_POLICY = 'disposable-fortress-no-checkpoint'
MAX_STEPS = 8
MAX_ORIGINAL_STEPS = 128
MAX_INVENTORY = 262144
MAX_REVIEW = 8192
MAX_TICK = (2**32 - 1) * 403200 + 403199
REGION_KEYS = ('x', 'y', 'z', 'width', 'height')
Guard = Callable[[], None]


def _idle() -> None:
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def digest(domain: bytes, value: object) -> str:
    return hashlib.sha256(domain + b'\0' + canonical(value)).hexdigest()


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'invalid prefix integer')
    return value


def identity(value: object) -> str:
    require(type(value) is str and re.fullmatch('[0-9a-f]{64}', value) is not None,
            'invalid prefix identity')
    return value


def _region(value: object) -> tuple[int, ...]:
    require(type(value) is dict and set(value) == set(REGION_KEYS), 'invalid original dig region')
    x, y, z, width, height = (integer(value[key], 1, 32767) for key in REGION_KEYS)
    require(width <= 8 and height <= 8 and x + width <= 32767
            and y + height <= 32767 and z <= 32766, 'original region or halo exceeds bounds')
    return x, y, z, width, height


def inventory_identity(state: dict) -> str:
    return digest(b'dfmcp-dig-blueprint-inventory/1',
                  {key: value for key, value in state.items() if key != 'inventory_digest'})


def _inventory(state: object, guard: Guard) -> None:
    """Shape/digest checks supplement, never replace, the effect owner's replay."""
    guard()
    require(type(state) is dict, 'complete original inventory required')
    required = {'batch_id', 'layout_digest', 'blueprint_digest', 'inventory_digest',
                'phase', 'stopped', 'total_steps', 'target_tiles', 'designated_steps',
                'next_step', 'unresolved_step', 'minimum_tick', 'minimum_sequence', 'records',
                'excavation_completion_proven', 'current_terrain_proven',
                'global_controller_fence', 'checkpoint_verified', 'retry_commit_permitted'}
    require(required <= set(state) <= required | {'room_excavation', 'batch_format'},
            'unknown or missing original inventory fields')
    for key in ('batch_id', 'layout_digest', 'blueprint_digest', 'inventory_digest'):
        identity(state[key])
    total = integer(state['total_steps'], 1, MAX_ORIGINAL_STEPS)
    integer(state['target_tiles'], 1, 512)
    integer(state['minimum_tick'], 0, 2**64 - 2)
    integer(state['minimum_sequence'], 0, 2**64 - 2)
    require(type(state['stopped']) is bool, 'invalid original stop flag')
    for key in ('excavation_completion_proven', 'current_terrain_proven',
                'global_controller_fence', 'checkpoint_verified', 'retry_commit_permitted'):
        require(state[key] is False, 'inventory grants unsupported proof or retry authority')
    rows = state['records']
    require(type(rows) is list and len(rows) <= total, 'original inventory exceeds step bound')
    for index, row in enumerate(rows):
        guard()
        require(type(row) is dict and set(row) == {'step', 'key', 'region', 'state',
            'witness', 'plan_digest', 'intent_sha256', 'terminal_receipt_sha256'},
            'invalid original step record')
        require(type(row['step']) is int and row['step'] == index
                and row['key'] == f"bp-{state['batch_id']}-{index:03d}",
                'original prefix key or order changed')
        _region(row['region'])
        require(row['state'] in ('designated', 'refused', 'unknown')
                and (index == len(rows) - 1 or row['state'] == 'designated'),
                'unresolved or refused original step was bypassed')
        for key in ('witness', 'plan_digest', 'intent_sha256'):
            identity(row[key])
        if row['state'] == 'unknown':
            require(row['terminal_receipt_sha256'] is None, 'unknown step claims terminal evidence')
        else:
            identity(row['terminal_receipt_sha256'])
    phase = ('unresolved' if rows[-1]['state'] == 'unknown' else 'refused') if rows and rows[-1]['state'] != 'designated' else (
        'designations_verified' if len(rows) == total else 'ready')
    require(state['phase'] == phase, 'original inventory phase disagrees with complete history')
    designated = sum(row['state'] == 'designated' for row in rows)
    require(type(state['designated_steps']) is int and state['designated_steps'] == designated,
            'original designated count changed')
    next_step = len(rows) if phase == 'ready' and not state['stopped'] else None
    unresolved = len(rows) - 1 if phase == 'unresolved' else None
    for key, expected in (('next_step', next_step), ('unresolved_step', unresolved)):
        require(state[key] is None if expected is None else
                type(state[key]) is int and state[key] == expected, 'original frontier disagrees')
    require(len(canonical(state)) <= MAX_INVENTORY
            and state['inventory_digest'] == inventory_identity(state), 'original inventory digest changed')
    guard()


@dataclass(frozen=True)
class Prefix:
    """A normalized locally rebuilt confirmation; imported reviews grant nothing."""
    _body: bytes

    @classmethod
    def create(cls, state: dict, regions: list[dict], count: int, *,
               allow_hidden_neighbors: bool, checkpoint_policy: str,
               guard: Guard = _idle) -> Prefix:
        _inventory(state, guard)
        integer(count, 1, MAX_STEPS)
        require(type(allow_hidden_neighbors) is bool and checkpoint_policy == CHECKPOINT_POLICY,
                'prefix cannot replace original risk or checkpoint policy')
        require(state['phase'] == 'ready' and not state['stopped'],
                'original batch is stopped, finished or unresolved; do not run')
        require(type(regions) is list and len(regions) == state['total_steps'],
                'complete original layout required')
        mask = set()
        for index, region in enumerate(regions):
            guard()
            x, y, z, width, height = _region(region)
            require(len(mask) + width * height <= 512, 'original target allowance exceeded')
            cells = {(xx, yy, z) for yy in range(y, y + height) for xx in range(x, x + width)}
            require(not mask & cells, 'original native regions overlap')
            mask.update(cells)
            if index < len(state['records']):
                require(state['records'][index]['region'] == region, 'record changed original targets')
        require(len(mask) == state['target_tiles'], 'original target count changed')
        start = state['next_step']
        require(start + count <= len(regions), 'requested prefix exceeds remaining original work')
        body = {'schema': SCHEMA, 'policy': POLICY, 'native_profile': 'dig/1.16',
                **{key: state[key] for key in ('batch_id', 'layout_digest', 'blueprint_digest', 'inventory_digest')},
                'original_steps': len(regions), 'original_target_tiles': len(mask),
                'start_step': start, 'step_count': count,
                'steps': [{'step': index, 'key': f"bp-{state['batch_id']}-{index:03d}",
                           'region': regions[index]} for index in range(start, start + count)],
                'allow_hidden_neighbors': allow_hidden_neighbors, 'checkpoint_policy': checkpoint_policy,
                'confirmation_scope': 'exact_original_prefix_with_fresh_per_step_native_plans',
                'one_shot_connection_per_step': True, 'automatic_recovery_or_retry': False,
                'atomic': False, 'native_terrain_observed': False,
                'mutation_authority_granted': False, 'excavation_completion_proven': False}
        raw = canonical(body)
        require(len(raw) <= MAX_REVIEW, 'complete prefix review exceeds bound')
        guard()
        return cls(raw)

    @property
    def seal(self) -> str:
        return hashlib.sha256(b'dfmcp-dig-blueprint-prefix-review/1\0' + self._body).hexdigest()

    def json(self) -> dict:
        return {**json.loads(self._body), 'confirm_run': self.seal}

    def confirm(self, batch_id: str, seal: str) -> None:
        identity(batch_id)
        identity(seal)
        require(batch_id == self.json()['batch_id'] and seal == self.seal,
                'stale or altered prefix confirmation; inspect the original batch')

    def successor(self, before: dict, after: dict, guard: Guard = _idle) -> str:
        """Only exactly one verified next record permits another original step."""
        _inventory(before, guard)
        _inventory(after, guard)
        value = self.json()
        index = before['next_step']
        require(type(index) is int and value['start_step'] <= index < value['start_step'] + value['step_count'],
                'execution escaped the reviewed prefix')
        require(not before['stopped'] and before['phase'] == 'ready', 'original prefix cannot advance')
        if index == value['start_step']:
            require(before['inventory_digest'] == value['inventory_digest'],
                    'first dispatch differs from the reviewed inventory')
        dynamic = {'phase', 'designated_steps', 'next_step', 'unresolved_step', 'minimum_tick',
                   'minimum_sequence', 'records', 'inventory_digest'}
        require({k: v for k, v in before.items() if k not in dynamic}
                == {k: v for k, v in after.items() if k not in dynamic},
                'run changed original batch identity, goal or policy')
        require(all(before[key] == value[key] for key in ('batch_id', 'layout_digest', 'blueprint_digest')),
                'run belongs to a different original batch')
        require(len(after['records']) == len(before['records']) + 1
                and after['records'][:-1] == before['records'], 'run skipped, repeated or rewrote an original effect')
        expected = value['steps'][index - value['start_step']]
        row = after['records'][-1]
        require(all(row[key] == expected[key] for key in ('step', 'key', 'region')),
                'run substituted an original key or target')
        require(after['minimum_tick'] >= before['minimum_tick']
                and after['minimum_sequence'] >= before['minimum_sequence'], 'run source clock regressed')
        guard()
        return row['state']
