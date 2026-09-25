"""Bounded, immutable evidence for the native excavation-run/1.18 profile.

Checksums identify evidence; they do not authorize effects or prove continuous
stability, mining causality, present pause, or production admission.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import re
import struct

MAX_TICK = (2**32 - 1) * 403200 + 403199
MAX_U64 = 2**64 - 1
PHASES = ('prepared', 'running', 'stopping', 'stopped', 'refused', 'source_lost')
REASONS = ('none', 'tick_limit', 'wall_limit', 'cancelled', 'external_pause',
           'native_failure', 'clock_regression', 'source_changed', 'shutdown', 'stale')
TRIGGERS = ('none', 'floor_observed', 'source_changed', 'capture_failure',
            'unobservable', 'liquid_observed')


class Rejected(ValueError):
    """A bounded protocol/custody refusal, never proof of nonapplication."""


def require(ok: bool, message: str) -> None:
    if not ok:
        raise Rejected(message)


def integer(value: object, low: int, high: int) -> int:
    require(type(value) is int and low <= value <= high, 'integer outside profile bounds')
    return value


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def digest(domain: str, raw: bytes) -> bytes:
    return hashlib.sha256(domain.encode('ascii') + b'\0' + raw).digest()


def exact_hex(value: object, size: int | None = None, maximum: int = 3072) -> bytes:
    require(type(value) is str and 0 < len(value) <= maximum * 2 and len(value) % 2 == 0
            and re.fullmatch('[0-9a-f]+', value) is not None, 'noncanonical hex')
    require(size is None or len(value) == size * 2, 'wrong hex width')
    return bytes.fromhex(value)


def key_text(key: str) -> bytes:
    require(type(key) is str and re.fullmatch('[A-Za-z0-9_.-]{1,128}', key) is not None,
            'invalid idempotency key')
    raw = key.encode('ascii')
    return struct.pack('>H', len(raw)) + raw


def text(raw: bytes, maximum: int) -> str:
    require(type(raw) is bytes and 1 <= len(raw) <= maximum and b'\0' not in raw,
            'invalid bounded text')
    try:
        return raw.decode('utf-8', errors='strict')
    except UnicodeError as error:
        raise Rejected('invalid UTF-8') from error


class Reader:
    def __init__(self, raw: bytes, maximum: int):
        require(type(raw) is bytes and len(raw) <= maximum, 'oversized/nonbyte record')
        self.raw, self.offset = raw, 0

    def take(self, size: int) -> bytes:
        require(0 <= size <= len(self.raw) - self.offset, 'truncated record')
        start = self.offset
        self.offset += size
        return self.raw[start:self.offset]

    def number(self, width: int) -> int:
        return int.from_bytes(self.take(width), 'big')

    def flag(self) -> bool:
        return bool(integer(self.number(1), 0, 1))

    def bounded(self, maximum: int) -> bytes:
        size = integer(self.number(2), 1, maximum)
        return self.take(size)

    def finish(self) -> None:
        require(self.offset == len(self.raw), 'trailing record bytes')


@dataclass(frozen=True)
class Region:
    x: int
    y: int
    z: int
    width: int
    height: int

    def __post_init__(self):
        for value in (self.x, self.y, self.z):
            integer(value, 0, 32767)
        integer(self.width, 1, 8)
        integer(self.height, 1, 8)
        require(self.x + self.width <= 32768 and self.y + self.height <= 32768,
                'region outside tile coordinate bounds')

    def values(self) -> tuple[int, ...]:
        return self.x, self.y, self.z, self.width, self.height


@dataclass(frozen=True)
class Capture:
    raw: bytes
    generation: int
    sequence: int
    tick: int
    paused: bool
    site: int
    dimensions: tuple[int, int, int]
    folder: str
    region: Region
    # Redacted cells are one-element tuples; there is no hidden payload.
    cells: tuple[tuple[int, ...], ...]

    @classmethod
    def decode(cls, raw: bytes) -> Capture:
        r = Reader(raw, 1024)
        require(r.take(8) == b'DFMEC018' and r.take(8) == b'DFMRO013', 'wrong capture profile')
        generation = integer(r.number(8), 1, MAX_U64 - 1)
        sequence, tick = r.number(8), integer(r.number(8), 0, MAX_TICK)
        loaded, valid, paused = r.flag(), r.flag(), r.flag()
        require(loaded and valid, 'capture has no usable clock')
        site = integer(r.number(4), 0, 2**31 - 1)
        dimensions = tuple(integer(r.number(4), 1, 32768) for _ in range(3))
        folder = text(r.bounded(512), 512)
        region = Region(*(r.number(4) for _ in range(5)))
        require(region.x + region.width <= dimensions[0] and region.y + region.height <= dimensions[1]
                and region.z < dimensions[2], 'region outside captured map')
        require(r.number(2) == region.width * region.height, 'wrong cell count')
        cells = []
        for _ in range(region.width * region.height):
            presence = integer(r.number(1), 0, 2)
            cells.append((presence, integer(r.number(1), 0, 8), integer(r.number(1), 0, 7),
                          integer(r.number(1), 0, 7)) if presence == 2 else (presence,))
        r.finish()
        return cls(raw, generation, sequence, tick, paused, site, dimensions, folder, region, tuple(cells))

    @property
    def identity(self) -> tuple:
        return self.generation, self.site, self.dimensions, self.folder

    @property
    def matches(self) -> bool:
        return all(cell == (2, 3, 0, 0) for cell in self.cells)

    def view(self) -> dict:
        return {'capture_digest': hashlib.sha256(self.raw).hexdigest(),
                'generation': self.generation, 'sequence': self.sequence, 'tick': self.tick,
                'paused': self.paused, 'site': self.site, 'folder': self.folder,
                'dimensions': list(self.dimensions), 'region': list(self.region.values()),
                'visible_cells': sum(c[0] == 2 for c in self.cells),
                'hidden_cells': sum(c[0] == 1 for c in self.cells),
                'missing_cells': sum(c[0] == 0 for c in self.cells),
                'matching_cells': sum(c == (2, 3, 0, 0) for c in self.cells),
                'floor_condition_observed': self.matches}


@dataclass(frozen=True)
class Spec:
    game_ticks: int
    wall_ms: int
    samples: int
    stable_ticks: int
    interval: int
    max_gap: int

    def __post_init__(self):
        integer(self.game_ticks, 1, 1200)
        integer(self.wall_ms, 1, 60000)
        integer(self.samples, 1, 128)
        integer(self.stable_ticks, 0, self.game_ticks)
        integer(self.interval, 1, self.game_ticks)
        integer(self.max_gap, self.interval, 1200)
        require(self.interval + max((self.samples - 1) * self.interval, self.stable_ticks) < self.game_ticks,
                'stability window cannot fit strictly inside run horizon')

    def values(self) -> tuple[int, ...]:
        return self.game_ticks, self.wall_ms, self.samples, self.stable_ticks, self.interval, self.max_gap

    def encode(self) -> bytes:
        return struct.pack('>6I', *self.values())


@dataclass(frozen=True)
class Plan:
    key: str
    spec: Spec
    before: Capture

    def __post_init__(self):
        key_text(self.key)
        require(type(self.spec) is Spec and type(self.before) is Capture, 'invalid typed plan')
        # Public dataclass construction cannot substitute fields for the sealed bytes.
        require(Capture.decode(self.before.raw) == self.before, 'capture fields differ from sealed bytes')
        require(self.before.paused and not self.before.matches, 'start requires paused unsatisfied goal')
        require(self.before.sequence < MAX_U64 and self.before.tick <= MAX_TICK - self.spec.game_ticks,
                'exhausted source sequence or clock horizon')
        require(all(c[0] == 2 and c[2] == 0 for c in self.before.cells), 'start requires visible dry targets')

    @property
    def digest(self) -> bytes:
        return digest('dfmcp-excavation-run-plan/1', self.spec.encode() + self.before.raw)

    @property
    def token(self) -> bytes:
        return digest('dfmcp-excavation-run-token/1', key_text(self.key) + self.digest)[:16]


@dataclass(frozen=True)
class Record:
    raw: bytes
    plan: Plan
    phase: str
    reason: str
    attempted: bool
    pause_verified: bool
    observed_tick: int | None
    trigger: str
    stable_samples: int
    first_stable_tick: int
    counted_tick: int
    last_tick: int
    sample: Capture | None

    @classmethod
    def decode(cls, raw: bytes, expected: Plan | None = None) -> Record:
        r = Reader(raw, 3072)
        require(r.take(8) == b'DFMER018', 'wrong effect profile')
        key = text(r.bounded(128), 128)
        spec = Spec(*(r.number(4) for _ in range(6)))
        plan = Plan(key, spec, Capture.decode(r.bounded(1024)))
        require(expected is None or plan == expected, 'effect belongs to another plan')
        require(r.take(32) == plan.digest and r.take(16) == plan.token, 'effect plan/token mismatch')
        phase, reason = integer(r.number(1), 0, 5), integer(r.number(1), 0, 9)
        attempted, verified, known = r.flag(), r.flag(), r.flag()
        tick = r.number(8)
        require((known and tick <= MAX_TICK) or (not known and tick == 0), 'invalid observed tick')
        trigger = integer(r.number(1), 0, 5)
        stable = integer(r.number(4), 0, spec.samples)
        first, counted, last = (r.number(8) for _ in range(3))
        sample = Capture.decode(r.bounded(1024)) if r.flag() else None
        checksum_start = r.offset
        require(r.take(32) == digest('dfmcp-excavation-run-receipt/1', raw[:checksum_start]),
                'effect integrity mismatch')
        r.finish()
        require(attempted == (phase in (1, 2, 3, 5)) and verified == (phase == 3), 'impossible effect flags')
        allowed = ((0,), (0,), (1, 2, 3, 5, 6, 8), (1, 2, 3, 4, 5, 6, 8), (3, 7, 9), (7,))
        require(reason in allowed[phase], 'impossible phase/reason')
        require(phase not in (0, 4, 5) or not known, 'terminal/prepared tick must be unknown')
        require(phase != 1 or (known and tick >= plan.before.tick), 'invalid running clock')
        require(plan.before.tick <= counted <= last, 'invalid sample clock order')
        require((stable and plan.before.tick < first <= counted) or (not stable and first == 0),
                'invalid stable-window origin')
        if sample is not None:
            require(sample.region == plan.before.region and sample.identity == plan.before.identity
                    and sample.sequence == plan.before.sequence + 1 and not sample.paused
                    and sample.tick == last and last < plan.before.tick + spec.game_ticks,
                    'sample identity/window differs from plan')
        else:
            require(stable == 0 and last == counted == plan.before.tick, 'missing sample payload')
        require(phase not in (0, 4) or (sample is None and trigger == 0), 'predispatch record claims sampling')
        require(trigger == 0 or phase in (2, 3, 5), 'trigger without stopping')
        if stable:
            require(sample is not None and sample.matches and first >= plan.before.tick + spec.interval
                    and counted - first >= (stable - 1) * spec.interval,
                    'impossible sampled stability evidence')
        if trigger == 1:
            require(sample is not None and sample.matches and stable == spec.samples
                    and counted == last and counted - first >= spec.stable_ticks,
                    'floor trigger lacks the required observed window')
        if trigger in (1, 3, 4, 5):
            require(phase == 5 or reason == 3, 'goal/safety trigger did not request cancellation')
        if trigger == 2:
            require(phase == 5, 'source-change trigger without source loss')
        if trigger in (3, 4, 5):
            require(stable == 0, 'failed sampling retained a positive streak')
        if trigger == 4:
            require(sample is not None and any(c[0] != 2 for c in sample.cells), 'unobservable trigger has no witness')
        if trigger == 5:
            require(sample is not None and all(c[0] == 2 for c in sample.cells)
                    and any(c[2] != 0 for c in sample.cells), 'liquid trigger has no witness')
        return cls(raw, plan, PHASES[phase], REASONS[reason], attempted, verified,
                   tick if known else None, TRIGGERS[trigger], stable, first, counted, last, sample)

    @property
    def terminal(self) -> bool:
        return self.phase in ('stopped', 'refused', 'source_lost')

    @property
    def resolved(self) -> bool:
        return self.phase in ('stopped', 'refused')

    def view(self) -> dict:
        advanced = (self.observed_tick - self.plan.before.tick if self.observed_tick is not None
                    and self.observed_tick >= self.plan.before.tick else None)
        return {'key': self.plan.key, 'plan_digest': self.plan.digest.hex(), 'phase': self.phase,
                'reason': self.reason, 'trigger': self.trigger, 'unpause_attempted': self.attempted,
                'historical_pause_verified': self.pause_verified, 'observed_tick': self.observed_tick,
                'observed_ticks_advanced': advanced,
                'observed_tick_overshoot': max(0, advanced - self.plan.spec.game_ticks) if advanced is not None else None,
                'stable_samples': self.stable_samples, 'first_stable_tick': self.first_stable_tick,
                'counted_tick': self.counted_tick, 'last_capture_tick': self.last_tick,
                'sample': None if self.sample is None else self.sample.view(),
                'receipt_digest': self.raw[-32:].hex(), 'terminal': self.terminal,
                'operator_attention_required': self.phase == 'source_lost',
                'sampled_floor_condition_reported': self.trigger == 'floor_observed',
                'current_pause_proved': False, 'continuous_stability_proved': False,
                'mining_causality_proved': False, 'retry_permitted': False}


def successor(previous: Record, current: Record) -> None:
    """Reject contradictory retained history without inventing missing samples."""
    require(previous.plan == current.plan, 'retained plan changed')
    if previous.terminal:
        require(previous.raw == current.raw, 'terminal receipt changed')
        return
    allowed = {'prepared': PHASES, 'running': ('running', 'stopping', 'stopped', 'source_lost'),
               'stopping': ('stopping', 'stopped', 'source_lost')}
    require(current.phase in allowed[previous.phase], 'native phase regressed')
    if previous.trigger != 'none':
        require(current.trigger == previous.trigger, 'native stop trigger changed')
    if previous.phase == 'stopping':
        require(current.phase == 'source_lost' or current.reason == previous.reason, 'native stop reason changed')
    require(current.last_tick >= previous.last_tick, 'retained sample regressed')
    if previous.sample is not None and current.last_tick == previous.last_tick:
        # Same-tick captures can reset a streak or report a hazard, not add samples.
        require(current.stable_samples <= previous.stable_samples, 'same-tick sample inflation')
    if previous.observed_tick is not None and current.phase == 'running':
        require(current.observed_tick >= previous.observed_tick, 'running clock regressed')
