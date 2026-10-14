"""Original-batch custody and context for the unchanged run/1.13 protocol.

A scope is immutable evidence/intent, never authority. The owner supplies current
query or effect authority on every boundary. No new native method or retry path.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import os
from pathlib import Path

import bounded_run_client as clock
import bounded_run_outcomes as storage
from furniture_plan import canonical, decode_json, integer, label, require
from route_furniture_review import Source, digest_text

POLICY = 'exact-clock-and-original-batch-one-bounded-progress-effect/2'
SCHEMA = 'dfmcp.route-furniture-progress/2'
SCOPE_SUFFIX = '.scope.json'
SCOPE_KEYS = {'schema', 'policy', 'batch_id', 'review_digest', 'plan_digest',
              'inventory_digest', 'next_step', 'frontier_evidence', 'source',
              'game_ticks', 'wall_ms', 'clock_manifest', 'clock_observation_hex'}


@dataclass(frozen=True)
class Scope:
    raw: bytes

    @property
    def digest(self):
        return clock.digest(b'dfmcp-route-furniture-progress-confirmation/2', self.raw).hex()

    def value(self):
        require(type(self.raw) is bytes and 0 < len(self.raw) < storage.MAX_FILE,
                'progress scope exceeds its byte bound')
        value = decode_json(self.raw)  # Bounds depth and duplicate fields before general parsing.
        require(type(value) is dict and set(value) == SCOPE_KEYS
                and canonical(value) == self.raw and value['schema'] == SCHEMA
                and value['policy'] == POLICY, 'invalid canonical progress scope')
        for key in ('batch_id', 'review_digest', 'plan_digest', 'inventory_digest', 'frontier_evidence'):
            digest_text(value[key])
        label(value['next_step'])
        integer(value['game_ticks'], 1, 1200)
        integer(value['wall_ms'], 1, 60000)
        source = Source.from_json(value['source'])
        manifest = value['clock_manifest']
        require(type(manifest) is dict and set(manifest) == {'generation', 'df_version', 'dfhack_version'},
                'invalid selected clock manifest')
        integer(manifest['generation'], 1, 2**64 - 2)
        require((manifest['df_version'], manifest['dfhack_version']) ==
                (source.df_version, source.dfhack_version), 'clock and route software differ')
        before = clock.snapshot(clock.exact_hex(value['clock_observation_hex'], 35))
        require(before['generation'] == manifest['generation'] and before['tick'] == source.tick
                and before['loaded'] and before['clock_valid'] and before['paused'],
                'clock differs from the reviewed loaded paused route frontier')
        return value

    @classmethod
    def create(cls, value):
        # Values here come from bounded native/route owners, not arbitrary input.
        out = cls(canonical(value))
        out.value()
        return out

    def bind(self, batch, *, current=False):
        value = self.value()
        require(value['batch_id'] == batch.id and value['review_digest'] == batch.review.digest
                and value['plan_digest'] == batch.plan.digest,
                'clock scope belongs to another original batch or review')
        require(value['next_step'] in {step.name for step in batch.plan.steps},
                'clock scope names a foreign original step')
        source = Source.from_json(value['source'])
        require(clock.endpoint(source.endpoint) == batch.address, 'clock scope endpoint changed')
        batch.source.permits(source)
        state = batch.audit()
        require(state['inventory_verified'] is True, 'original batch custody unverified')
        if current:
            require(state['inventory_digest'] == value['inventory_digest']
                    and state['next_step'] == value['next_step']
                    and state['advance_allowed'] is True and not state['stopped'],
                    'original clock-progress frontier changed')
        return value

    def intent(self):
        value = self.value()
        raw = clock.exact_hex(value['clock_observation_hex'], 35)
        key = 'frp-' + self.digest
        plan = clock.plan_for(value['game_ticks'], value['wall_ms'], raw)
        return clock.verify_intent({
            'format': 'dfmcp.bounded-run-intent/1', 'endpoint': value['source']['endpoint'],
            'idempotency_key': key, 'game_ticks': value['game_ticks'], 'wall_ms': value['wall_ms'],
            'observation_hex': raw.hex(), 'plan_digest_hex': plan.hex(),
            'prepare_token_hex': clock.token_for(key, plan).hex(),
            'df_version': value['clock_manifest']['df_version'],
            'dfhack_version': value['clock_manifest']['dfhack_version'],
            'effect_status': 'indeterminate_until_native_query'})


class ScopeStore:
    """Immutable association under the caller's already-held clock directory lock.

    Sync before native intent/preparation. Existing unassociated v1 records are
    never adopted. Orphan/torn files remain intact and cannot permit redispatch.
    """
    def __init__(self, path: Path, parent: int, batch, expected: str, *, create: Scope | None = None):
        digest_text(expected)
        self.path, self.parent, self.batch = path, parent, batch
        self.budget = batch.budget
        self.expected, self.create = expected, create
        self.name, self.fd, self.data, self.scope = path.name + SCOPE_SUFFIX, None, None, None
        self.intent_stamp = None

    def _parent(self):
        fd = storage._open_parent(self.path)
        try:
            a, b = os.fstat(fd), os.fstat(self.parent)
            require((a.st_dev, a.st_ino) == (b.st_dev, b.st_ino), 'clock scope parent replaced')
        finally:
            os.close(fd)

    def __enter__(self):
        try:
            self.batch.budget.remaining()
            self._parent()
            if self.create is not None:
                require(type(self.create) is Scope and self.create.digest == self.expected,
                        'scope differs from independently confirmed identity')
                self.create.bind(self.batch, current=True)
                data = self.create.raw + b'\n'
                self.batch.budget.reserve('disk_bytes', len(data))
                self.fd = os.open(self.name, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
                                  | os.O_CLOEXEC | os.O_NONBLOCK, 0o600, dir_fd=self.parent)
                storage._private(os.fstat(self.fd))
                left = memoryview(data)
                while left:
                    self.batch.budget.work()
                    n = os.write(self.fd, left)
                    require(n > 0, 'short progress scope write')
                    left = left[n:]
                os.fsync(self.fd)
                os.fsync(self.parent)
            else:
                self.fd = os.open(self.name, storage.OPEN_READ, dir_fd=self.parent)
                data = None
            self.batch.budget.reserve('disk_bytes', storage.MAX_FILE)
            self.data = storage._read(self.fd, self.parent, self.name)
            require(data is None or data == self.data, 'progress scope changed after synchronization')
            require(self.data.endswith(b'\n'), 'incomplete progress scope')
            self.scope = Scope(self.data[:-1])
            self.scope.bind(self.batch)
            require(self.scope.digest == self.expected, 'progress scope confirmation mismatch')
            self.stamp = storage._fingerprint(os.fstat(self.fd))
            self.check()
            return self
        except BaseException:
            self.close()
            raise

    def check(self):
        require(self.batch.budget is self.budget, 'clock scope budget owner changed')
        self.batch.budget.work()
        self.batch.budget.reserve('disk_bytes', storage.MAX_FILE)
        self._parent()
        require(storage._fingerprint(os.fstat(self.fd)) == self.stamp
                and storage._read(self.fd, self.parent, self.name) == self.data,
                'original progress association changed')
        self.scope.bind(self.batch)

    def verify_intent(self, intent):
        """Re-read the original synchronized capsule at EACH native effect."""
        self.check()
        self.batch.budget.reserve('disk_bytes', storage.MAX_FILE)
        fd = os.open(self.path.name, storage.OPEN_READ, dir_fd=self.parent)
        try:
            data = storage._read(fd, self.parent, self.path.name)
            stamp = storage._fingerprint(os.fstat(fd))
            require(self.intent_stamp is None or stamp == self.intent_stamp,
                    'original synchronized intent identity changed before effect')
            self.intent_stamp = stamp
        finally:
            os.close(fd)
        expected = canonical({'intent': intent, 'sha256':
            hashlib.sha256(canonical(intent)).hexdigest()}) + b'\n'
        require(data == expected, 'durable clock intent differs before native effect')

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __exit__(self, error_type, *_):
        try:
            if error_type is None:
                self.check()
        finally:
            self.close()


class Client(clock.Client):
    """Actual native framing with shared counters/deadline and boundary guards.

    Review connections cannot mutate. Execution consumes one exact preparation
    and commit; recovery can only query the immutable original intent. Callback
    parameters are trusted embedding ports, never command-line/native selectors.
    """
    def __init__(self, authority, budget, guard, *, intent=None, query=False, effect_guard=None):
        require(type(query) is bool, 'query mode requires an exact Boolean')
        self.authority, self.budget, self.guard = authority, budget, guard
        self.intent = None if intent is None else clock.verify_intent(dict(intent))
        require(not query or self.intent is not None, 'clock recovery requires an original intent')
        self.query, self.effect_guard = query, effect_guard
        self.prepare_attempted = self.commit_attempted = self.prepared = False
        self.operation_deadline = budget.deadline
        self._check()
        require(self.intent is None or clock.endpoint(self.intent['endpoint']) == authority.address,
                'native clock endpoint differs from original intent')
        timeout = min(60000, int(budget.remaining() * 1000))
        integer(timeout, 1, 60000)
        super().__init__(authority.address, authority.run_token, timeout)
        self.deadline = min(self.deadline, self.operation_deadline)

    def _check(self):
        self.authority.clock_guard()
        require(self.budget.deadline == self.operation_deadline, 'foreground deadline changed')
        self.budget.remaining()
        self.guard()

    def remaining(self):
        self._check()
        return min(super().remaining(), self.budget.remaining())

    def send(self, data):
        self._check()
        self.budget.reserve('network_bytes', len(data))
        super().send(data)

    def read(self, size):
        self._check()
        integer(size, 0, 65536)
        self.budget.reserve('network_bytes', size)
        return super().read(size)

    def frame(self, method, request):
        self._check()
        self.budget.reserve('calls', 1)
        return super().frame(method, request)

    def call(self, operation, fields=None):
        try:
            self._check()
            if self.intent is None:
                require(operation in ('Handshake', 'ObserveRun'), 'clock review cannot mutate or recover')
            elif self.query:
                require(operation in ('Handshake', 'QueryRun'), 'clock recovery is query-only')
            else:
                require(operation in ('Handshake', 'ObserveRun', 'PrepareRun', 'CommitRun'),
                        'clock start cannot retry or cancel')
            if operation in ('PrepareRun', 'CommitRun', 'QueryRun'):
                expected = clock.intent_fields(self.intent, operation == 'CommitRun')
                if operation == 'PrepareRun':
                    expected.update({6: self.intent['game_ticks'], 7: self.intent['wall_ms'],
                                     8: bytes.fromhex(self.intent['observation_hex'])})
                    require(not self.prepare_attempted, 'clock preparation already attempted')
                    self.prepare_attempted = True
                elif operation == 'CommitRun':
                    require(self.prepared and not self.commit_attempted, 'clock commit unavailable or already attempted')
                    self.commit_attempted = True
                require(fields == expected, 'clock request differs from original confirmed intent')
                if operation in ('PrepareRun', 'CommitRun'):
                    require(self.effect_guard is not None, 'clock effect lacks durable scope guard')
                    self.effect_guard(self.intent)
                    self._check()
            result = super().call(operation, fields)
            if self.intent is not None:
                clock.match_record(result, self.intent)
                original = clock.snapshot(bytes.fromhex(self.intent['observation_hex']))
                # Recovery may return historical stopped/source_lost evidence
                # from a later incarnation, never a renewed write permission.
                if not self.query and operation != 'CommitRun':
                    require(result['manifest']['generation'] == original['generation'],
                            'clock incarnation differs from reviewed source')
                if operation == 'ObserveRun':
                    require(result['observation_hex'] == self.intent['observation_hex']
                            and not result['owner_active'] and result['retained_records'] < 256,
                            'clock observation or owner changed since review')
            if operation == 'PrepareRun':
                self.prepared = result.get('record', {}).get('phase') == 'prepared'
            self._check()  # Revocation during a reply cannot unlock the next effect.
            return result
        except BaseException:
            self.close()
            raise
