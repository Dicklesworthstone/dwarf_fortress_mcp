"""Private custody for original route batches, reusing native placement journals.

Closed root inventory: batch.json, index.jsonl, effects/, optional stop.json.
The index pins every child intent and inode before native preparation. Reopen
never restores a placement permit. No overwrite, repair, eviction or retry.
Beads: df-dfhack-bridge-plane-c-pic.4/.5.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import secrets

import build_placement_store as storage
from build_placement_wire import Capture, exact_hex
from furniture_plan import canonical, integer, progress, require, unique
from route_furniture_review import Review, digest_text
from route_furniture_frontier import bind_native_selection, step_key

SCHEMA = 'dfmcp.route-furniture-batch/1'
ROOM_SCHEMA = 'dfmcp.room-route-furniture-batch/1'
MAX_DEFINITION = 128 * 1024
MAX_INDEX = 32768
HEADER = b'{"schema":"dfmcp.route-furniture-index/1"}\n'


def sha(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def identity(fd: int) -> list[int]:
    info = os.fstat(fd)
    return [info.st_dev, info.st_ino]


def seal(value: dict) -> bytes:
    return canonical({'value': value, 'sha256': sha(canonical(value))}) + b'\n'


def unseal(raw: bytes, budget) -> dict:
    require(type(raw) is bytes and 0 < len(raw) <= MAX_DEFINITION, 'batch record exceeds bound')
    depth, quoted, escaped = 0, False, False
    for i, byte in enumerate(raw):
        if i % 128 == 0:
            budget.work()
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
            require(depth <= 16, 'batch record nesting exceeds bound')
        elif byte in (93, 125):
            depth -= 1
            require(depth >= 0, 'invalid batch record shape')
    require(depth == 0 and not quoted, 'incomplete batch record')
    wrapped = storage.exact_object(json.loads(raw, object_pairs_hook=unique), {'value', 'sha256'})
    require(type(wrapped['value']) is dict and seal(wrapped['value']) == raw,
            'batch record checksum or canonical bytes differ')
    return wrapped['value']


class File:
    def __init__(self, root: int, name: str, budget, writable: bool, *, initial=None, maximum=MAX_INDEX):
        self.root, self.name, self.budget, self.maximum, self.fd = root, name, budget, maximum, None
        try:
            flags = os.O_RDWR | os.O_APPEND if writable else os.O_RDONLY
            if initial is not None:
                require(writable and 0 < len(initial) <= maximum, 'invalid initial batch file')
                flags |= os.O_CREAT | os.O_EXCL
            self.fd = os.open(name, flags | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                              0o600, dir_fd=root)
            self.identity = identity(self.fd)
            storage.private(os.fstat(self.fd), 0o600)
            if initial is not None:
                budget.reserve('disk_bytes', len(initial))
                storage.write_all(self.fd, initial)
                os.fsync(self.fd)
                os.fsync(root)
            self.raw = self.read()
            require(initial is None or initial == self.raw, 'batch file publication changed')
        except BaseException:
            self.close()
            raise

    def read(self) -> bytes:
        self.budget.remaining()
        before = os.fstat(self.fd)
        storage.private(before, 0o600)
        require(identity(self.fd) == self.identity and 0 < before.st_size <= self.maximum,
                'batch file identity or extent changed')
        self.budget.reserve('disk_bytes', before.st_size + 1)
        raw = bytearray()
        while len(raw) <= before.st_size:
            self.budget.work()
            part = os.pread(self.fd, min(4096, before.st_size + 1 - len(raw)), len(raw))
            if not part:
                break
            raw += part
        named = os.stat(self.name, dir_fd=self.root, follow_symlinks=False)
        storage.private(named, 0o600)
        require(storage.stamp(before) == storage.stamp(os.fstat(self.fd)) == storage.stamp(named)
                and len(raw) == before.st_size, 'batch bytes or path changed during read')
        self.budget.remaining()
        return bytes(raw)

    def check(self) -> None:
        require(self.read() == self.raw, 'batch file bytes changed')

    def close(self) -> None:
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None


class Batch:
    def __init__(self, path: str, expected_id: str | None, budget, *, writable=False, initial=None):
        self.path, self.budget, self.writable = path, budget, writable
        self.fd, self.effects, self.files, self.fenced = None, None, {}, False
        try:
            budget.remaining()
            require((initial is not None) == (expected_id is None), 'select original batch or exclusive initialization')
            if expected_id is not None:
                digest_text(expected_id)
            self.fd = storage.open_directory(path)
            import fcntl
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.root_identity = identity(self.fd)
            if initial is not None:
                require(writable and not self.names(), 'initialization requires an empty private directory')
                origin, manifest, before = initial
                # New room-backed roots retain the COMPLETE parent, not an
                # optional sidecar that an older owner could silently ignore.
                # Decode before creating any file or effect directory.
                room_handoff = None
                if type(origin) is Review:
                    review = origin
                else:
                    from room_route_handoff import RoomRouteHandoff
                    require(type(origin) is RoomRouteHandoff, 'unsupported route batch origin')
                    room_handoff = RoomRouteHandoff.decode(origin.raw, origin.digest, budget.work)
                    _, review = room_handoff.unpack(budget.work)
                original, request, source, plan = review.unpack(budget.work)
                require(bind_native_selection(review, before, budget.work) == plan.ordered[0].name,
                        'initial native capture is not the first original selection')
                require(manifest.generation == before.generation
                        and (manifest.df_version, manifest.dfhack_version) == (source.df_version, source.dfhack_version),
                        'initial native source differs from original route source')
                value = {'schema': SCHEMA, 'nonce': secrets.token_hex(24), 'review': original,
                         'review_digest': review.digest, 'root_identity': self.root_identity,
                         'effects_identity': [2**64-1, 2**64-1],
                         'native_manifest': manifest.view(), 'native_before_hex': before.raw.hex()}
                if room_handoff is not None:
                    # Do not duplicate the child review or raise storage limits.
                    value.pop('review')
                    value.pop('review_digest')
                    value.update(schema=ROOM_SCHEMA,
                        room_handoff=room_handoff.json(budget.work),
                        room_handoff_digest=room_handoff.digest)
                require(len(seal(value)) <= MAX_DEFINITION, 'complete route batch is too large')
                budget.remaining()
                os.mkdir('effects', 0o700, dir_fd=self.fd)
                effects = storage.open_directory(str(Path(path) / 'effects'))
                try:
                    value['effects_identity'] = identity(effects)
                finally:
                    os.close(effects)
                os.fsync(self.fd)
                for name, raw, maximum in (('batch.json', seal(value), MAX_DEFINITION),
                                           ('index.jsonl', HEADER, MAX_INDEX)):
                    self.files[name] = File(self.fd, name, budget, True, initial=raw, maximum=maximum)
            names = self.names()
            require({'batch.json', 'index.jsonl', 'effects'} <= names
                    and names <= {'batch.json', 'index.jsonl', 'effects', 'stop.json'},
                    'incomplete or foreign route batch inventory')
            for name in sorted(names - {'effects'} - self.files.keys()):
                self.files[name] = File(self.fd, name, budget, writable and name == 'index.jsonl',
                                        maximum=MAX_DEFINITION if name == 'batch.json' else MAX_INDEX)
            self.id = sha(self.files['batch.json'].raw)
            require(expected_id is None or expected_id == self.id, 'selected original batch digest differs')
            value = unseal(self.files['batch.json'].raw, budget)
            require(value.get('schema') in (SCHEMA, ROOM_SCHEMA), 'unsupported route batch generation')
            room_backed = value['schema'] == ROOM_SCHEMA
            fields = {'room_handoff', 'room_handoff_digest'} if room_backed else {'review', 'review_digest'}
            storage.exact_object(value, {'schema', 'nonce', 'root_identity', 'effects_identity',
                                         'native_manifest', 'native_before_hex'} | fields)
            exact_hex(value['nonce'], 24)
            for name in ('root_identity', 'effects_identity'):
                require(type(value[name]) is list and len(value[name]) == 2, 'invalid directory identity')
                for number in value[name]:
                    integer(number, 0, 2**64-1)
            require(value['root_identity'] == self.root_identity, 'route batch moved or copied')
            self.room_handoff = None
            if room_backed:
                from room_route_handoff import RoomRouteHandoff
                self.room_handoff = RoomRouteHandoff.decode(canonical(value['room_handoff']),
                    value['room_handoff_digest'], budget.work)
                _, self.review = self.room_handoff.unpack(budget.work)
            else:
                self.review = Review.decode(canonical(value['review']), value['review_digest'], budget.work)
            _, self.request, self.source, self.plan = self.review.unpack(budget.work)
            self.native = storage.manifest_from(value['native_manifest'])
            self.before = Capture.decode(exact_hex(value['native_before_hex'], maximum=2048))
            require(self.before.generation == self.native.generation
                    and (self.native.df_version, self.native.dfhack_version) ==
                    (self.source.df_version, self.source.dfhack_version), 'native binding differs')
            require(bind_native_selection(self.review, self.before, budget.work) == self.plan.ordered[0].name,
                    'initial native selection differs')
            self.address = storage.endpoint(self.source.endpoint)
            self.value = value
            self.effects = storage.PlacementDirectory(str(Path(path) / 'effects'), budget, writable)
            require(list(self.effects.identity) == value['effects_identity'], 'route effects directory replaced')
            self.entries = self.decode_index(self.files['index.jsonl'].raw)
            self.stopped = 'stop.json' in self.files
            if self.stopped:
                require(unseal(self.files['stop.json'].raw, budget) ==
                        {'schema': 'dfmcp.route-furniture-stop/1', 'batch_id': self.id}, 'invalid stop record')
            self.audit()
        except BaseException:
            self.close()
            raise

    def names(self) -> set[str]:
        out = set()
        with os.scandir(self.fd) as entries:
            for entry in entries:
                self.budget.work()
                require(len(out) < 4, 'route batch has foreign files')
                out.add(entry.name)
        return out

    def check(self) -> None:
        require(not self.fenced and self.fd is not None, 'route batch fenced or closed')
        try:
            self.budget.remaining()
            fd = storage.open_directory(self.path)
            try:
                require(identity(fd) == self.root_identity == identity(self.fd), 'route batch pathname replaced')
            finally:
                os.close(fd)
            require(self.names() == set(self.files) | {'effects'}, 'route batch membership changed')
            for file in self.files.values():
                file.check()
            self.effects.check()
        except BaseException:
            self.fenced = True
            raise

    def key(self, step) -> str:
        require(step in self.plan.steps, 'selection outside the original route batch')
        return step_key(self.id, step.name)

    def decode_index(self, raw: bytes) -> list[dict]:
        require(raw.startswith(HEADER) and raw.endswith(b'\n') and len(raw) <= MAX_INDEX, 'torn route index')
        lines = raw[len(HEADER):].splitlines(keepends=True)
        require(len(lines) <= len(self.plan.steps), 'route index exceeds original plan')
        out, previous = [], sha(HEADER)
        for step, line in zip(self.plan.ordered, lines):
            value = unseal(line, self.budget)
            storage.exact_object(value, {'slot', 'key', 'child_identity', 'intent_sha256', 'route_evidence_digest', 'previous'})
            require(value['slot'] == step.name and value['key'] == self.key(step)
                    and value['previous'] == previous, 'route index order or key differs')
            digest_text(value['intent_sha256'])
            if value['route_evidence_digest'] is not None:
                digest_text(value['route_evidence_digest'])
            require(type(value['child_identity']) is list and len(value['child_identity']) == 2,
                    'invalid route child identity')
            for number in value['child_identity']:
                integer(number, 0, 2**64-1)
            out.append(value)
            previous = sha(line)
        return out

    def bind(self, before, manifest) -> None:
        require(manifest == self.native and before.generation == self.native.generation
                and before.tick >= self.before.tick and before.sequence >= self.before.sequence
                and before.next_job >= self.before.next_job and before.next_building >= self.before.next_building,
                'route native source or frontier regressed')
        bind_native_selection(self.review, before, self.budget.work)

    def audit(self) -> dict:
        self.check()
        expected = {storage.filename(self.key(step)) for step in self.plan.steps}
        require(set(self.effects.journals) <= expected, 'foreign native work in route batch')
        indexed = {row['slot']: row for row in self.entries}
        recorded, details, previous = {}, {}, None
        for step in self.plan.ordered:
            self.budget.work()
            journal = self.effects.journals.get(storage.filename(self.key(step)))
            require(journal is not None or step.name not in indexed, 'registered route child disappeared')
            if journal is None:
                continue
            state = journal.state
            self.bind(state.plan.before, state.manifest)
            require(state.address == self.address and state.plan.key == self.key(step)
                    and state.plan.before.selection.item == step.item
                    and state.plan.before.selection.target == step.target, 'route child selection differs')
            if previous is not None:
                before = state.plan.before
                require(before.tick >= previous.tick and before.sequence >= previous.sequence
                        and before.next_job >= previous.next_job and before.next_building >= previous.next_building,
                        'route child predates a predecessor')
            if step.name in indexed:
                entry = indexed[step.name]
                require(entry['child_identity'] == identity(journal.fd)
                        and entry['intent_sha256'] == sha(journal.raw.splitlines(keepends=True)[0]),
                        'route child intent or inode changed')
            record = state.terminal
            recorded[step.name] = record.phase if record else 'unknown'
            details[step.name] = {'key': state.plan.key, 'registered': step.name in indexed,
                'plan_digest': state.plan.digest.hex(), 'dispatch_recorded': state.dispatched,
                'receipt_digest': record.raw[-32:].hex() if record else None,
                'route_evidence_digest': indexed.get(step.name, {}).get('route_evidence_digest')}
            if record is not None and record.after is not None:
                previous = record.after
        out = progress(self.plan, recorded)
        for row in out['steps']:
            row.update(details.get(row['name'], {}))
        out.update(schema='dfmcp.route-furniture-batch-state/1', batch_id=self.id,
            review_digest=self.review.digest, original_review=self.review.json(self.budget.work),
            inventory_verified=True, stopped=self.stopped,
            advance_allowed=out['status'] == 'ready' and not self.stopped
                and all(row.get('registered', True) for row in out['steps']),
            route_constraints_checked_this_call=False, atomic_route_placement_proven=False)
        if self.room_handoff is not None:
            room, original_review = self.room_handoff.unpack(self.budget.work)
            require(original_review.raw == self.review.raw and original_review.digest == self.review.digest,
                    'room-backed batch changed its original route review')
            out.update(room_handoff_digest=self.room_handoff.digest, room_plan_digest=room.digest,
                original_room_plan=room.json(), complete_original_room_intent_retained=True,
                room_constraints_checked_this_call=False, room_completion_proven=False,
                atomic_room_placement_proven=False)
        out['inventory_digest'] = sha(canonical({'batch': self.id, 'index': sha(self.files['index.jsonl'].raw),
            'children': [[name, sha(j.raw)] for name, j in sorted(self.effects.journals.items())], 'stopped': self.stopped}))
        self.check()
        return out

    def placed_prefix(self) -> tuple[bytes, ...]:
        self.audit()
        out = []
        for step in self.plan.ordered:
            journal = self.effects.journals.get(storage.filename(self.key(step)))
            if journal is None or journal.state.terminal is None or journal.state.terminal.phase != 'placed':
                break
            out.append(journal.state.terminal.raw)
        return tuple(out)

    def register(self, step, evidence: str | None) -> None:
        require(self.writable, 'route index is read-only')
        if evidence is not None:
            digest_text(evidence)
        self.audit()
        if step.name in {row['slot'] for row in self.entries}:
            return
        require(self.plan.ordered[len(self.entries)] == step, 'route registration is not the next child')
        journal = self.effects.get(self.key(step))
        file = self.files['index.jsonl']
        line = seal({'slot': step.name, 'key': self.key(step), 'child_identity': identity(journal.fd),
            'intent_sha256': sha(journal.raw.splitlines(keepends=True)[0]), 'route_evidence_digest': evidence,
            'previous': sha(file.raw.splitlines(keepends=True)[-1])})
        raw = file.raw + line
        entries = self.decode_index(raw)
        self.check()
        self.budget.reserve('disk_bytes', len(line))
        try:
            storage.write_all(file.fd, line)
            os.fsync(file.fd)
            os.fsync(self.fd)
            require(file.read() == raw, 'route index append readback differs')
            file.raw, self.entries = raw, entries
            self.check()
        except BaseException:
            self.fenced = True
            raise

    def stop(self) -> dict:
        self.audit()
        require(self.writable, 'route stop requires writable custody')
        if not self.stopped:
            try:
                self.files['stop.json'] = File(self.fd, 'stop.json', self.budget, True,
                    initial=seal({'schema': 'dfmcp.route-furniture-stop/1', 'batch_id': self.id}))
                self.stopped = True
            except BaseException:
                self.fenced = True
                raise
        return self.audit()

    def close(self) -> None:
        try:
            if self.effects is not None:
                self.effects.close()
                self.effects = None
        finally:
            for file in self.files.values():
                file.close()
            self.files.clear()
            if self.fd is not None:
                os.close(self.fd)
                self.fd = None

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()
