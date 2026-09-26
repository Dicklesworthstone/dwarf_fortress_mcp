"""Exact, bounded furniture action DAGs; geometry is not placement authority.

Every step names one existing item and one fixed target. No item search, native
read, implied wall clearing, coordinate translation, or replacement selection.
"""
from __future__ import annotations

from dataclasses import dataclass, field
import hashlib
import json
import re

SCHEMA = 'dfmcp.furniture-plan/1'
MAX_STEPS = 32
MAX_BYTES = 16384
KINDS = ('bed', 'chair', 'table')


def require(ok: bool, message: str) -> None:
    if not ok:
        raise ValueError(message)


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'furniture integer outside bounds')
    return value


def label(value: object) -> str:
    require(type(value) is str and re.fullmatch(r'[A-Za-z0-9_.-]{1,48}', value) is not None,
            'step names must be 1..48 ASCII letters, digits, dot, underscore or hyphen')
    return value


def unique(pairs: list[tuple[str, object]]) -> dict:
    out = {}
    for key, value in pairs:
        require(key not in out, 'duplicate furniture JSON field')
        out[key] = value
    return out


def decode_json(raw: bytes) -> object:
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_BYTES, 'furniture plan byte bound exceeded')
    depth, quoted, escaped = 0, False, False
    for byte in raw:
        if quoted:
            if escaped:
                escaped = False
            elif byte == 92:
                escaped = True
            elif byte == 34:
                quoted = False
        elif byte == 34:
            quoted = True
        elif byte in (91, 123):
            depth += 1
            require(depth <= 8, 'furniture plan nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1
    return json.loads(raw.decode('utf-8'), object_pairs_hook=unique)


@dataclass(frozen=True)
class Step:
    name: str
    kind: str
    item: int
    target: tuple[int, int, int]
    after: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        label(self.name)
        require(type(self.kind) is str and self.kind in KINDS, 'unsupported furniture kind')
        integer(self.item, 0, 2147483646)
        require(type(self.target) is tuple and len(self.target) == 3, 'target must have three coordinates')
        for coordinate in self.target[:2]:
            integer(coordinate, 1, 32766)  # Full native same-level 3x3 context.
        integer(self.target[2], 0, 32767)
        require(type(self.after) is tuple and len(self.after) < MAX_STEPS,
                'invalid dependency collection')
        for dependency in self.after:
            label(dependency)
        require(len(set(self.after)) == len(self.after) and self.name not in self.after,
                'duplicate or self dependency')
        object.__setattr__(self, 'after', tuple(sorted(self.after)))

    def json(self) -> dict:
        return {'name': self.name, 'kind': self.kind, 'item': self.item,
                'target': list(self.target), 'after': list(self.after)}

    @classmethod
    def from_json(cls, value: object) -> Step:
        require(type(value) is dict and {'name', 'kind', 'item', 'target'} <= set(value)
                and set(value) <= {'name', 'kind', 'item', 'target', 'after'}, 'invalid furniture step fields')
        require(type(value['target']) is list and type(value.get('after', [])) is list,
                'invalid furniture target or dependency array')
        return cls(value['name'], value['kind'], value['item'], tuple(value['target']),
                   tuple(value.get('after', [])))


@dataclass(frozen=True)
class FurniturePlan:
    steps: tuple[Step, ...]
    ordered: tuple[Step, ...] = field(init=False)

    def __post_init__(self) -> None:
        require(type(self.steps) is tuple and 1 <= len(self.steps) <= MAX_STEPS
                and all(type(step) is Step for step in self.steps), 'plan requires 1..32 exact steps')
        require(len({s.name for s in self.steps}) == len(self.steps), 'duplicate furniture step name')
        require(len({s.item for s in self.steps}) == len(self.steps), 'one item cannot furnish two targets')
        require(len({s.target for s in self.steps}) == len(self.steps), 'two furnishings share one target')
        by_name = {s.name: s for s in self.steps}
        require(all(d in by_name for s in self.steps for d in s.after), 'unresolved furniture dependency')
        # Canonical Kahn order: select the lexically first ready step each time.
        # Bounds make quadratic readiness checks small and explicit (<=1024).
        done, ordered = set(), []
        while len(done) < len(self.steps):
            ready = sorted(name for name, step in by_name.items()
                           if name not in done and set(step.after) <= done)
            require(bool(ready), 'cyclic furniture dependencies')
            name = ready[0]
            done.add(name)
            ordered.append(by_name[name])
        object.__setattr__(self, 'steps', tuple(sorted(self.steps, key=lambda s: s.name)))
        object.__setattr__(self, 'ordered', tuple(ordered))
        require(len(canonical(self.json())) <= MAX_BYTES, 'normalized furniture plan exceeds byte bound')

    def json(self) -> dict:
        return {'schema': SCHEMA, 'steps': [step.json() for step in self.steps]}

    @property
    def digest(self) -> str:
        return hashlib.sha256(b'dfmcp-furniture-plan/1\0' + canonical(self.json())).hexdigest()

    @classmethod
    def from_json(cls, value: object) -> FurniturePlan:
        require(type(value) is dict and set(value) == {'schema', 'steps'}
                and value['schema'] == SCHEMA and type(value['steps']) is list
                and 1 <= len(value['steps']) <= MAX_STEPS, 'invalid furniture plan envelope')
        return cls(tuple(Step.from_json(step) for step in value['steps']))

    @classmethod
    def decode(cls, raw: bytes) -> FurniturePlan:
        return cls.from_json(decode_json(raw))

    def check_dimensions(self, dimensions: object) -> None:
        require(type(dimensions) in (list, tuple) and len(dimensions) == 3, 'invalid map dimensions')
        for size in dimensions:
            integer(size, 1, 32768)
        for step in self.steps:
            x, y, z = step.target
            require(x + 1 < dimensions[0] and y + 1 < dimensions[1] and z < dimensions[2],
                    'furniture target or required context is outside observed map')


def progress(plan: FurniturePlan, recorded: dict[str, str]) -> dict:
    """Reduce a verified fixed-order prefix. Callers must authenticate receipts.

    Only historical Placed receipts unlock the next step. A terminal refusal or
    cancellation stops the batch too: it is not an excuse to weaken its intent.
    """
    require(type(recorded) is dict and set(recorded) <= {s.name for s in plan.steps},
            'unexpected furniture batch record')
    rows, status, next_step, pending = [], 'ready', None, None
    prefix_open = True
    for step in plan.ordered:
        phase = recorded.get(step.name)
        if phase is not None:
            require(prefix_open and phase in ('placed', 'refused', 'cancelled', 'unknown', 'indeterminate'),
                    'records are not one valid fixed-order batch prefix')
        if phase != 'placed':
            if prefix_open:
                if phase is None:
                    next_step = step.name
                elif phase in ('unknown', 'indeterminate'):
                    status, pending = 'pending_recovery', step.name
                else:
                    status = 'halted_' + phase
            prefix_open = False
        rows.append({'name': step.name, 'phase': phase or 'not_started',
                     'dependencies': list(step.after)})
    if prefix_open:
        status = 'all_placed'
    return {'status': status, 'next_step': next_step, 'pending_step': pending,
            'placed': sum(value == 'placed' for value in recorded.values()),
            'total': len(plan.steps), 'steps': rows, 'atomic': False,
            'construction_completion_proven': False, 'retry_permitted': False}
