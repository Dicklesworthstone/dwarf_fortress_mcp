#!/usr/bin/env python3
"""Resume exact furnishing plans through the existing one-shot placement client.

One reviewed native placement per advance; no retry, substitution, unpause or
atomic-plan claim. Query/cancel recover original child journals, never commit.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import secrets
import sys

from room_furniture_handoff import RoomFurnitureHandoff, MAX_BYTES as MAX_ROOM_HANDOFF_BYTES
from terrain_furniture_handoff import TerrainFurnitureHandoff, MAX_BYTES as MAX_TERRAIN_HANDOFF_BYTES
from room_terrain_origin import ReadBudget

import furniture_plan as model
from furniture_handoff import Handoff, MAX_BYTES as MAX_HANDOFF_BYTES
from furniture_allocation import Candidate
import build_placement_client as placement
import build_placement_store as storage
from build_placement_rpc import Authority, Budget, Client, Reply, endpoint
from build_placement_wire import Plan, Selection, canonical, exact_hex, integer, require, text_bytes, MAX_TICK

SCHEMA = 'dfmcp.furniture-batch/1'
HANDOFF_SCHEMA = 'dfmcp.furniture-batch/2'
MAX_DEFINITION = 65536
ROOM_SCHEMA = 'dfmcp.furniture-batch/3'
TERRAIN_SCHEMA = 'dfmcp.furniture-batch/4'
MAX_TERRAIN_DEFINITION = 384 * 1024
MAX_ROOM_DEFINITION = 128 * 1024
MAX_ROOM_OUTPUT = 192 * 1024
HEADER = b'{"schema":"dfmcp.furniture-batch-index/1"}\n'
MAX_FILE = 32768
MAX_OUTPUT = 65536


def sha(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def identity(fd: int) -> list[int]:
    info = os.fstat(fd)
    return [info.st_dev, info.st_ino]


def seal(value: dict) -> bytes:
    return canonical({'value': value, 'sha256': sha(canonical(value))}) + b'\n'


def unseal(raw: bytes) -> dict:
    envelope = storage.exact_object(json.loads(raw, object_pairs_hook=model.unique), {'value', 'sha256'})
    require(type(envelope['value']) is dict and seal(envelope['value']) == raw, 'invalid batch checksum or encoding')
    return envelope['value']


def selection(step: model.Step) -> Selection:
    return Selection(placement.KINDS[step.kind], step.item, *step.target)


def validate_handoff_capture(handoff: Handoff, capture, manifest, address: str) -> None:
    """Retained inventory only narrows an independently decoded native observation."""
    from build_placement_wire import Capture
    require(type(handoff) is Handoff and type(capture) is Capture, 'invalid handoff capture')
    capture.encode()
    require(capture.generation == manifest.generation, 'placement capture generation differs')
    handoff.validate_binding(address, capture.folder, capture.site,
                             manifest.df_version, manifest.dfhack_version)
    step = next((s for s in handoff.plan().steps if selection(s) == capture.selection), None)
    require(step is not None and capture.item.presence == 2, 'original selected item is not observed')
    item = capture.item
    candidate = Candidate(step.item, step.kind, item.pos,
                          (item.material, item.material_index), item.subtype)
    require(item.kind == capture.selection.kind, 'native selected furniture kind changed')
    handoff.validate_item(step, candidate, item.native_type, capture.tick,
                          capture.next_job, capture.next_building)


class File:
    """One bounded descriptor-pinned batch metadata file, never a native journal."""
    def __init__(self, root: int, name: str, budget: Budget, writable: bool, initial: bytes | None = None,
                 *, maximum: int = MAX_FILE):
        self.root, self.name, self.budget = root, name, budget
        self.maximum = integer(maximum, 1, MAX_TERRAIN_DEFINITION)
        self.fd = None
        try:
            flags = os.O_RDWR | os.O_APPEND if writable else os.O_RDONLY
            if initial is not None:
                require(writable and 0 < len(initial) <= self.maximum, 'invalid batch publication')
                flags |= os.O_CREAT | os.O_EXCL
            self.fd = os.open(name, flags | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, 0o600, dir_fd=root)
            self.identity = identity(self.fd)
            if initial is not None:
                storage.private(os.fstat(self.fd), 0o600)
                budget.remaining()
                storage.write_all(self.fd, initial)
                os.fsync(self.fd)
                os.fsync(root)
            self.raw = self.read()
            require(initial is None or self.raw == initial, 'batch publication readback differs')
        except BaseException:
            self.close()
            raise

    def read(self) -> bytes:
        self.budget.remaining()
        before = os.fstat(self.fd)
        storage.private(before, 0o600)
        require(identity(self.fd) == self.identity and 0 < before.st_size <= self.maximum, 'invalid batch file identity or extent')
        raw = bytearray()
        while len(raw) <= before.st_size:
            self.budget.remaining()
            part = os.pread(self.fd, before.st_size + 1 - len(raw), len(raw))
            if not part:
                break
            raw += part
        after = os.fstat(self.fd)
        named = os.stat(self.name, dir_fd=self.root, follow_symlinks=False)
        storage.private(named, 0o600)
        require(storage.stamp(before) == storage.stamp(after) == storage.stamp(named)
                and len(raw) == before.st_size, 'batch file changed or was substituted')
        return bytes(raw)

    def check(self) -> None:
        require(self.read() == self.raw, 'batch file bytes changed')

    def close(self) -> None:
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None


@contextmanager
def root_lock(path: str, budget: Budget):
    budget.remaining()
    fd = storage.open_directory(path)
    try:
        import fcntl
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield fd
    finally:
        os.close(fd)


def names(fd: int) -> set[str]:
    out = set()
    with os.scandir(fd) as entries:
        for entry in entries:
            require(len(out) < 4, 'unexpected batch directory inventory')
            out.add(entry.name)
    return out


class Batch:
    def __init__(self, path: str, budget: Budget, writable: bool = False):
        self.path, self.budget, self.writable = path, budget, writable
        self.files, self.effects, self.fenced = {}, None, False
        self._terrain_context, self._terrain_journal = None, None
        self.lock = root_lock(path, budget)
        self.fd = self.lock.__enter__()
        try:
            self.root_identity = identity(self.fd)
            found = names(self.fd)
            require({'batch.json', 'steps.jsonl', 'effects'} <= found
                    and found <= {'batch.json', 'steps.jsonl', 'effects', 'stop.json'}, 'incomplete or foreign batch inventory')
            for name in sorted(found - {'effects'}):
                self.files[name] = File(self.fd, name, budget, writable and name == 'steps.jsonl',
                                        maximum=MAX_TERRAIN_DEFINITION if name == 'batch.json' else MAX_FILE)
            raw_definition = self.files['batch.json'].raw
            # Bound the containing envelope before general JSON parsing. Each
            # legacy profile still enforces its original byte ceiling below.
            check_definition_depth(raw_definition, budget, terrain=True)
            value = unseal(raw_definition)
            require(value.get('schema') in (SCHEMA, HANDOFF_SCHEMA, ROOM_SCHEMA, TERRAIN_SCHEMA), 'unsupported batch format')
            with_terrain = value['schema'] == TERRAIN_SCHEMA
            with_room = value['schema'] in (ROOM_SCHEMA, TERRAIN_SCHEMA)
            if not with_terrain:
                check_definition_depth(raw_definition, budget)
            with_handoff = value['schema'] == HANDOFF_SCHEMA
            storage.exact_object(value, {'schema', 'nonce', 'plan', 'source', 'endpoint', 'folder', 'site',
                                        'dimensions', 'first_tick', 'root_identity', 'effects_identity'}
                                 | ({'terrain_handoff'} if with_terrain else {'handoff'} if with_handoff
                                    else {'room_handoff'} if with_room else set()))
            maximum = (MAX_TERRAIN_DEFINITION if with_terrain else MAX_ROOM_DEFINITION if with_room
                       else MAX_DEFINITION if with_handoff else MAX_FILE)
            require(len(raw_definition) <= maximum, 'batch definition exceeds its profile bound')
            self.terrain_handoff = (TerrainFurnitureHandoff.from_json(value['terrain_handoff'],
                                    ReadBudget(budget).checkpoint) if with_terrain else None)
            self.room_handoff = (self.terrain_handoff.room if with_terrain else
                                 RoomFurnitureHandoff.from_json(value['room_handoff'], budget.remaining)
                                 if with_room else None)
            self.handoff = (self.room_handoff.allocation if with_room else
                            Handoff.from_json(value['handoff']) if with_handoff else None)
            exact_hex(value['nonce'], 24)
            self.plan = model.FurniturePlan.from_json(value['plan'])
            self.source = storage.manifest_from(value['source'])
            self.address = endpoint(value['endpoint'])
            text_bytes(value['folder'], 512)
            integer(value['site'], 0, 2147483647)
            integer(value['first_tick'], 0, MAX_TICK)
            self.plan.check_dimensions(value['dimensions'])
            if self.room_handoff is not None:
                require(self.plan.json() == value['plan'], 'original room batch plan is not normalized')
                self.room_handoff.check_dimensions(value['dimensions'], budget.remaining)
            if self.handoff is not None:
                self.handoff.validate_binding(value['endpoint'], value['folder'], value['site'],
                                              self.source.df_version, self.source.dfhack_version)
                require(self.plan == self.handoff.plan()
                        and value['first_tick'] >= self.handoff.source.tick,
                        'batch differs from retained allocation or predates its inventory')
            if self.terrain_handoff is not None:
                terrain = self.terrain_handoff
                require((value['endpoint'], value['folder'], value['site'], tuple(value['dimensions']),
                         self.source.df_version, self.source.dfhack_version) ==
                        (terrain.origin.json()['endpoint'], terrain.fresh.folder, terrain.fresh.site,
                         terrain.fresh.dimensions, terrain.fresh.manifest.df_version,
                         terrain.fresh.manifest.dfhack_version) and value['first_tick'] >= terrain.fresh.tick,
                        'batch source changed or predates retained terrain')
                require(not Path(terrain.origin.json()['journal_path']).is_relative_to(Path(path)),
                        'original terrain journal must remain outside the closed batch inventory')
            for field in ('root_identity', 'effects_identity'):
                require(type(value[field]) is list and len(value[field]) == 2, 'invalid persisted directory identity')
                for number in value[field]:
                    integer(number, 0, 2**64 - 1)
            require(value['root_identity'] == self.root_identity, 'batch directory moved or copied')
            self.value, self.id = value, sha(self.files['batch.json'].raw)
            self.effects = storage.PlacementDirectory(str(Path(path) / 'effects'), budget, writable)
            require(list(self.effects.identity) == value['effects_identity'], 'effects directory replaced')
            self.entries = self.decode_index(self.files['steps.jsonl'].raw)
            self.stopped = 'stop.json' in self.files
            if self.stopped:
                require(unseal(self.files['stop.json'].raw) == {'schema': 'dfmcp.furniture-batch-stop/1',
                                                              'batch_id': self.id}, 'invalid batch stop')
            self.audit()
        except BaseException:
            self.close()
            raise

    def decode_index(self, raw: bytes) -> list[dict]:
        require(raw.startswith(HEADER) and raw.endswith(b'\n') and len(raw) <= MAX_FILE, 'torn batch index')
        lines = raw[len(HEADER):].splitlines(keepends=True)
        require(len(lines) <= len(self.plan.steps), 'batch index too long')
        entries, previous = [], sha(HEADER)
        for index, line in enumerate(lines):
            self.budget.remaining()
            entry = unseal(line)
            storage.exact_object(entry, {'batch_id', 'step', 'intent_sha256', 'file_identity', 'previous'})
            require(entry['batch_id'] == self.id and entry['step'] == self.plan.ordered[index].name
                    and entry['previous'] == previous, 'batch index identity or order differs')
            exact_hex(entry['intent_sha256'], 32)
            require(type(entry['file_identity']) is list and len(entry['file_identity']) == 2, 'invalid child identity')
            for number in entry['file_identity']:
                integer(number, 0, 2**64 - 1)
            entries.append(entry)
            previous = sha(line)
        return entries

    def key(self, step: model.Step) -> str:
        # New keys bind the complete room definition, including room-only
        # geometry and exclusions. Legacy nonce keys are never migrated.
        if self.room_handoff is not None:
            require(step in self.plan.steps, 'step outside original room batch')
            return 'fr-' + self.id + '-' + step.name
        return 'fb-' + self.value['nonce'] + '-' + step.name

    def check(self) -> None:
        require(not self.fenced, 'batch fenced; reopen without repair')
        self.budget.remaining()
        pinned = storage.open_directory(self.path)
        try:
            require(identity(pinned) == self.root_identity == identity(self.fd), 'batch path replaced')
        finally:
            os.close(pinned)
        require(names(self.fd) == set(self.files) | {'effects'}, 'batch membership changed')
        for file in self.files.values():
            file.check()
        self.effects.check()
        require(list(self.effects.identity) == self.value['effects_identity'], 'effects directory changed')
        if self._terrain_journal is not None:
            # This operation already replayed and bound the complete immutable
            # history. Recheck all bytes/path custody, not just a cached digest.
            self._terrain_journal.verify()
            self._terrain_journal.budget.checkpoint()

    def require_terrain(self, journal=None) -> None:
        """Hold original terrain custody for NEW work; never call during recovery."""
        if self.terrain_handoff is None:
            require(journal is None, 'legacy batch cannot adopt terrain custody')
            return
        if self._terrain_journal is None:
            terrain = self.terrain_handoff
            if journal is None:
                context = terrain.origin.open(terrain.room.room_plan, self.budget)
                held = context.__enter__()
                self._terrain_context, self._terrain_journal = context, held
            else:
                require(type(journal.budget) is ReadBudget and journal.budget.parent is self.budget,
                        'terrain and batch must share their original operation budget')
                terrain.origin.verify(journal, terrain.room.room_plan, journal.budget.checkpoint)
                self._terrain_journal = journal  # Borrowed from initialization's outer owner.
        else:
            require(journal is None or journal is self._terrain_journal, 'terrain owner already bound')
        self.check()

    def bind(self, plan: Plan, manifest) -> None:
        self.bind_before(plan.before, manifest)

    def bind_before(self, capture, manifest) -> None:
        """Fresh review and retained child plans share the same original constraints."""
        self.bind_capture(capture, manifest)
        if self.handoff is not None:
            validate_handoff_capture(self.handoff, capture, manifest, self.value['endpoint'])
        if self.terrain_handoff is not None:
            self.terrain_handoff.bind_placement(capture, manifest, self.value['endpoint'])

    def bind_capture(self, capture, manifest) -> None:
        require(manifest == self.source and capture.generation == self.source.generation
                and capture.folder == self.value['folder'] and capture.site == self.value['site']
                and list(capture.dimensions) == self.value['dimensions']
                and capture.tick >= self.value['first_tick'], 'batch fortress, source or clock differs')

    def audit(self) -> dict:
        self.check()
        expected = {storage.filename(self.key(step)): step for step in self.plan.steps}
        require(set(self.effects.journals) <= set(expected), 'unplanned placement in batch directory')
        indexed = {entry['step']: entry for entry in self.entries}
        states, details = {}, {}
        last_after = None
        for step in self.plan.ordered:
            journal = self.effects.journals.get(storage.filename(self.key(step)))
            require(journal is not None or step.name not in indexed, 'registered child journal missing; cannot forget attempted work')
            if journal is None:
                continue
            state = journal.state
            self.bind(state.plan, state.manifest)
            require(state.address == self.address and state.plan.before.selection == selection(step), 'child differs from exact batch selection')
            if last_after is not None:
                before = state.plan.before
                require(before.tick >= last_after.tick and before.sequence >= last_after.sequence
                        and before.next_building >= last_after.next_building and before.next_job >= last_after.next_job,
                        'child clock, sequence or native ID horizon regressed')
            if step.name in indexed:
                entry = indexed[step.name]
                require(entry['intent_sha256'] == sha(journal.raw.splitlines(keepends=True)[0])
                        and entry['file_identity'] == identity(journal.fd), 'registered child intent or inode changed')
            record = state.terminal
            states[step.name] = record.phase if record is not None else 'unknown'
            details[step.name] = {'key': state.plan.key, 'plan_digest': state.plan.digest.hex(),
                'registered': step.name in indexed, 'receipt_digest': record.raw[-32:].hex() if record else None,
                'insertion': record.insertion.view() if record and record.insertion else None,
                'dispatch_recorded': state.dispatched}
            if record and record.after:
                last_after = record.after
        out = model.progress(self.plan, states)
        for row in out['steps']:
            row.pop('dependencies')  # Complete dependency graph remains in the immutable plan.
            row.update(details.get(row['name'], {}))
        out.update(batch_id=self.id, plan_digest=self.plan.digest, stopped=self.stopped,
                   inventory_verified=True, advance_allowed=out['status'] == 'ready' and not self.stopped
                   and all(row.get('registered', True) for row in out['steps']),
                   source=self.source.view(), endpoint=self.value['endpoint'], native_contacted=False)
        if self.handoff is not None:
            out['allocation'] = self.handoff.compact()
        if self.room_handoff is not None:
            out['room_origin'] = self.room_handoff.compact()
            out['room_plan'] = self.room_handoff.room_plan.json()
        if self.terrain_handoff is not None:
            out['terrain_origin'] = self.terrain_handoff.compact()
            out['terrain_history_verified_this_call'] = self._terrain_journal is not None
            out['advance_allowed'] &= self._terrain_journal is not None
            out['terrain_history_required_for_new_work'] = True
        out['inventory_digest'] = sha(canonical({'batch': self.id, 'index': sha(self.files['steps.jsonl'].raw),
            'children': [[name, sha(j.raw)] for name, j in sorted(self.effects.journals.items())], 'stopped': self.stopped}))
        self.check()
        return out

    def register(self, step: model.Step) -> None:
        require(self.writable, 'read-only batch index')
        self.audit()
        if step.name in {entry['step'] for entry in self.entries}:
            return
        require(len(self.entries) < len(self.plan.ordered) and self.plan.ordered[len(self.entries)] == step,
                'child registration is not the next original step')
        journal = self.effects.get(self.key(step))
        file = self.files['steps.jsonl']
        entry = {'batch_id': self.id, 'step': step.name,
                 'intent_sha256': sha(journal.raw.splitlines(keepends=True)[0]),
                 'file_identity': identity(journal.fd),
                 'previous': sha(file.raw.splitlines(keepends=True)[-1])}
        line = seal(entry)
        candidate = file.raw + line
        entries = self.decode_index(candidate)
        try:
            file.check()
            storage.write_all(file.fd, line)
            os.fsync(file.fd)
            os.fsync(self.fd)
            require(file.read() == candidate, 'batch index append differs')
            file.raw, self.entries = candidate, entries
            self.check()
        except BaseException:
            self.fenced = True
            raise

    def stop(self) -> dict:
        self.audit()
        if not self.stopped:
            require(self.writable, 'read-only batch stop')
            try:
                self.files['stop.json'] = File(self.fd, 'stop.json', self.budget, True,
                    seal({'schema': 'dfmcp.furniture-batch-stop/1', 'batch_id': self.id}))
                self.stopped = True
            except BaseException:
                self.fenced = True
                raise
        return self.audit()

    def close(self) -> None:
        context, self._terrain_context = self._terrain_context, None
        self._terrain_journal = None
        try:
            if self.effects is not None:
                self.effects.close()
                self.effects = None
            for file in self.files.values():
                file.close()
            self.files.clear()
            if self.fd is not None:
                self.lock.__exit__(None, None, None)
                self.fd = None
        finally:
            if context is not None:
                context.__exit__(*sys.exc_info())

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


def packet(operation: str, value: dict, ok: bool = True) -> dict:
    pending = value.get('pending_step')
    known = value.get('inventory_verified', False)
    return {'ok': ok, 'schema': 'dfmcp.furniture-batch-result/1', 'result': value,
            'agent_turn': {'schema': 'dfmcp.agent_turn/1', 'operation': 'furniture_batch.' + operation,
                'phase': 'reconcile' if pending or not known else 'propose' if operation == 'review' else 'inspect',
                'session_id': None, 'turn_id': None, 'request_id': None, 'anchor': None,
                'continuity': {'status': 'stale' if known else 'indeterminate', 'basis': None, 'gap': None},
                'profile': 'tactical', 'briefing': {'runtime_admitted': False, 'mutation_admissible': False},
                'changes': [], 'attention': [], 'active_work': {'inventory_verified': known,
                    'batch_id': value.get('batch_id'), 'pending_step': pending,
                    'remaining_steps': value.get('total', 0) - value.get('placed', 0) if known else None},
                'affordances': [], 'recommendations': [],
                'uncertainty': ['Receipts prove historical placement, not completed construction or current usability.',
                                'Batch custody is local, not a global controller fence or anti-rollback authority.'],
                'coverage': {'scope': 'exact_private_furnishing_batch', 'all_steps_listed': known},
                'budget': {'maximum_output_bytes': MAX_OUTPUT, 'token_count_measured': False},
                'references': [value['batch_id']] if known else []}}


def output_limit(out: dict) -> int:
    return MAX_ROOM_OUTPUT if 'room_origin' in out else MAX_OUTPUT


def encoded(operation: str, out: dict, ok: bool = True) -> bytes:
    value = packet(operation, out, ok)
    maximum = output_limit(out)
    if 'room_origin' in out:
        turn = value['agent_turn']
        turn['budget']['maximum_output_bytes'] = maximum
        turn['references'].append(out['room_origin'])
        turn['coverage']['original_room_intent'] = 'complete_retained_plan'
        turn['briefing'].update(room_completion_proven=False, terrain_completion_proven=False)
        if 'terrain_origin' in out:
            turn['references'].append(out['terrain_origin'])
            turn['coverage']['original_terrain_history_verified_this_call'] = out['terrain_history_verified_this_call']
            turn['uncertainty'].append('Original terrain completion is historical; retained map evidence is not current terrain or room-assignment proof.')
        else:
            turn['uncertainty'].append('Original room intent is retained, not terrain-completion or room-assignment evidence.')
    raw = canonical(value) + b'\n'
    require(len(raw) <= maximum, 'complete furnishing result exceeds its profile bound')
    return raw


def initialize(path: str, plan: model.FurniturePlan, folder: str, site: int, timeout_ms: int = 10000,
               *, handoff: Handoff | None = None, room_handoff: RoomFurnitureHandoff | None = None,
               terrain_handoff: TerrainFurnitureHandoff | None = None,
               _budget: Budget | None = None) -> dict:
    budget = _budget if _budget is not None else Budget(timeout_ms)
    if terrain_handoff is not None:
        require(type(terrain_handoff) is TerrainFurnitureHandoff and handoff is None and room_handoff is None,
                'terrain, room and standalone handoff imports are mutually exclusive')
        terrain = TerrainFurnitureHandoff.decode(terrain_handoff.encode(), ReadBudget(budget).checkpoint)
        require(not Path(terrain.origin.json()['journal_path']).is_relative_to(Path(path)),
                'original terrain journal must remain outside the closed batch inventory')
        with terrain.origin.open(terrain.room.room_plan, budget) as journal:
            out = _initialize(path, plan, folder, site, budget, room_handoff=terrain.room,
                              terrain_handoff=terrain, terrain_journal=journal)
            budget.publication_terrain = terrain
            return out
    return _initialize(path, plan, folder, site, budget, handoff=handoff, room_handoff=room_handoff)


def _initialize(path: str, plan: model.FurniturePlan, folder: str, site: int, budget: Budget,
                *, handoff: Handoff | None = None, room_handoff: RoomFurnitureHandoff | None = None,
                terrain_handoff: TerrainFurnitureHandoff | None = None, terrain_journal=None) -> dict:
    def terrain_guard() -> None:
        if terrain_journal is not None:
            terrain_journal.verify()
            terrain_journal.budget.checkpoint()
    terrain_guard()
    if room_handoff is not None:
        require(type(room_handoff) is RoomFurnitureHandoff and handoff is None,
                'room and standalone handoff imports are mutually exclusive')
        room_handoff = RoomFurnitureHandoff.decode(room_handoff.encode(), budget.remaining)
        handoff = room_handoff.allocation
    text_bytes(folder, 512)
    integer(site, 0, 2147483647)
    require(type(plan) is model.FurniturePlan, 'invalid complete furniture plan')
    if handoff is not None:
        require(type(handoff) is Handoff, 'invalid furniture handoff')
        handoff = Handoff.decode(canonical(handoff.json()))
        require(plan == handoff.plan() and (folder, site) == (handoff.request.folder, handoff.request.site),
                'explicit initialization differs from original request')
    budget.remaining()
    with root_lock(path, budget) as root:
        require(not names(root), 'initialization requires an existing empty private directory')
        authority = Authority.load()
        if room_handoff is not None:
            budget.publication_authority = authority
        address = f'{authority.address[0]}:{authority.address[1]}'
        if handoff is not None:
            require(address == handoff.source.address, 'operator endpoint differs from allocation')
        terrain_guard()
        with Client(authority, budget, selection(plan.ordered[0])) as client:
            reply = client.observe()
        capture = reply.capture
        require(capture.folder == folder and capture.site == site, 'initial observation is another fortress')
        plan.check_dimensions(capture.dimensions)
        if room_handoff is not None:
            room_handoff.check_dimensions(capture.dimensions, budget.remaining)
        if handoff is not None:
            validate_handoff_capture(handoff, capture, reply.manifest, address)
        if terrain_handoff is not None:
            terrain_handoff.bind_placement(capture, reply.manifest, address)
        terrain_guard()
        value = {'schema': TERRAIN_SCHEMA if terrain_handoff is not None else ROOM_SCHEMA if room_handoff is not None else SCHEMA if handoff is None else HANDOFF_SCHEMA,
            'nonce': secrets.token_hex(24), 'plan': plan.json(),
            'source': reply.manifest.view(), 'endpoint': address,
            'folder': folder, 'site': site, 'dimensions': list(capture.dimensions), 'first_tick': capture.tick,
            'root_identity': identity(root), 'effects_identity': [2**64 - 1, 2**64 - 1]}
        maximum = (MAX_TERRAIN_DEFINITION if terrain_handoff is not None else MAX_ROOM_DEFINITION
                   if room_handoff is not None else MAX_FILE if handoff is None else MAX_DEFINITION)
        if terrain_handoff is not None:
            value['terrain_handoff'] = terrain_handoff.json()
        elif room_handoff is not None:
            value['room_handoff'] = room_handoff.json()
        elif handoff is not None:
            value['handoff'] = handoff.json()
        # Reserve complete definition and visible original intent BEFORE creating custody.
        require(len(seal(value)) <= maximum, 'batch manifest too large')
        preview = model.progress(plan, {})
        if handoff is not None:
            preview['allocation'] = handoff.compact()
        if room_handoff is not None:
            preview['room_origin'] = room_handoff.compact()
            preview['room_plan'] = room_handoff.room_plan.json()
        if terrain_handoff is not None:
            preview['terrain_origin'] = terrain_handoff.compact()
            preview['terrain_history_verified_this_call'] = True
            preview['terrain_history_required_for_new_work'] = True
        require(len(encoded('init', preview)) + 8192 <= output_limit(preview), 'batch output reservation exceeds bound')
        authority.guard('QueryPlacement')
        budget.remaining()
        pinned = storage.open_directory(path)
        try:
            require(identity(pinned) == identity(root) and not names(root), 'initial batch directory changed')
        finally:
            os.close(pinned)
        terrain_guard()
        os.mkdir('effects', 0o700, dir_fd=root)
        effects = storage.open_directory(str(Path(path) / 'effects'))
        try:
            value['effects_identity'] = identity(effects)
        finally:
            os.close(effects)
        raw = seal(value)
        published = []
        try:
            for name, data in [('batch.json', raw), ('steps.jsonl', HEADER)]:
                authority.guard('QueryPlacement')
                terrain_guard()
                published.append(File(root, name, budget, True, data,
                                      maximum=maximum if name == 'batch.json' else MAX_FILE))
            for file in published:
                file.check()
        finally:
            for file in published:
                file.close()
    with Batch(path, budget) as batch:
        if terrain_handoff is not None:
            batch.require_terrain(terrain_journal)
        out = batch.audit()
        out['native_contacted'] = True
        encoded('init', out)
        authority.guard('QueryPlacement')
        budget.remaining()
        batch.check()
        return out


def select_next(batch: Batch, expected_id: str) -> tuple[model.Step, dict]:
    require(batch.id == expected_id, 'batch identity differs from explicit selection')
    out = batch.audit()
    require(out['advance_allowed'], 'batch stopped, finished, refused or pending original-key recovery')
    step = next(step for step in batch.plan.ordered if step.name == out['next_step'])
    return step, out


def review_seal(batch: Batch, step: model.Step, plan: Plan, inventory: str) -> str:
    return sha(b'dfmcp-furniture-step-review/1\0' + canonical({'batch_id': batch.id,
        'inventory': inventory, 'step': step.name, 'key': plan.key, 'native_plan': plan.digest.hex()}))


def review(path: str, expected_id: str, timeout_ms: int = 10000, *, _budget: Budget | None = None) -> dict:
    with Batch(path, _budget if _budget is not None else Budget(timeout_ms)) as batch:
        require(batch.id == expected_id, 'batch identity differs from explicit selection')
        batch.require_terrain()
        if batch.terrain_handoff is not None:
            batch.budget.publication_terrain = batch.terrain_handoff
        step, out = select_next(batch, expected_id)
        authority = Authority.load()
        if batch.room_handoff is not None:
            batch.budget.publication_authority = authority
        require(authority.address == batch.address, 'operator endpoint differs from original batch')
        with Client(authority, batch.budget, selection(step)) as client:
            reply = client.observe()
        # Even an ineligible capture is bound before its diagnostics are exposed.
        require(reply.manifest == batch.source and reply.capture.identity == (
            batch.source.generation, batch.value['site'], tuple(batch.value['dimensions']), batch.value['folder'])
            and reply.capture.tick >= batch.value['first_tick'], 'review source differs from batch')
        batch.bind_before(reply.capture, reply.manifest)
        out.update(native_contacted=True, before=reply.capture.view(),
                   blockers=list(reply.capture.blockers) + (['native_unresolved'] if reply.unresolved else [])
                   + (['native_retention_full'] if reply.retained_records == 256 else []),
                   expected_plan=None, confirm_review=None)
        if not out['blockers']:
            native_plan = Plan(batch.key(step), reply.capture)
            out['expected_plan'] = native_plan.digest.hex()
            out['confirm_review'] = review_seal(batch, step, native_plan, out['inventory_digest'])
        batch.audit()
        encoded('review', out)
        authority.guard('QueryPlacement')
        batch.budget.remaining()
        batch.check()
        return out


def advance(path: str, expected_id: str, expected_plan: str, confirmation: str, timeout_ms: int = 10000,
            *, _budget: Budget | None = None) -> dict:
    exact_hex(expected_plan, 32)
    exact_hex(confirmation, 32)
    with Batch(path, _budget if _budget is not None else Budget(timeout_ms), True) as batch:
        require(batch.id == expected_id, 'batch identity differs from explicit selection')
        batch.require_terrain()
        if batch.terrain_handoff is not None:
            batch.budget.publication_terrain = batch.terrain_handoff
        step, initial = select_next(batch, expected_id)
        require(len(encoded('advance', initial)) + 32768 < output_limit(initial), 'batch outcome reservation exceeds bound')
        authority = Authority.load(True)
        if batch.room_handoff is not None:
            batch.budget.publication_authority = authority
        require(authority.address == batch.address, 'operator endpoint differs from original batch')
        prior = {name: journal.raw for name, journal in batch.effects.journals.items()}
        prior_after = [j.state.terminal.after for j in batch.effects.journals.values()
                       if j.state.terminal is not None and j.state.terminal.after is not None]
        def guard(plan: Plan, known: Reply, stage: str) -> None:
            batch.bind(plan, known.manifest)
            require(all(plan.before.tick >= after.tick and plan.before.sequence >= after.sequence
                        and plan.before.next_building >= after.next_building
                        and plan.before.next_job >= after.next_job for after in prior_after),
                    'new capture regressed behind predecessor evidence')
            require(not known.unresolved and known.retained_records < 256, 'native fence or capacity blocks new intent')
            require(plan.key == batch.key(step) and plan.before.selection == selection(step), 'substituted batch step')
            state = batch.audit()
            require(not batch.stopped, 'batch stopped before native boundary')
            require(all(batch.effects.journals[name].raw == raw for name, raw in prior.items()), 'predecessor evidence changed')
            if stage == 'before_intent':
                require(state['inventory_digest'] == initial['inventory_digest'] and state['next_step'] == step.name
                        and confirmation == review_seal(batch, step, plan, state['inventory_digest']), 'stale or substituted batch review')
            else:
                journal = batch.effects.get(plan.key)
                require(journal.state.plan == plan and state['pending_step'] == step.name, 'current child is not the sole original pending step')
                if stage == 'before_prepare':
                    batch.register(step)  # File+directory sync precedes any native preparation.
                require(step.name in {entry['step'] for entry in batch.entries}, 'native work lacks registered child custody')
                batch.check()
        placement.start(batch.effects, authority, selection(step), batch.key(step), expected_plan, guard)
        out = batch.audit()
        out.update(native_contacted=True, advanced_step=step.name)
        encoded('advance', out)
        if batch.room_handoff is not None:
            authority.guard('QueryPlacement')
            batch.budget.remaining()
            batch.check()
        return out


def inspect(path: str, expected_id: str, step_name: str | None = None, timeout_ms: int = 10000,
            *, allocation: bool = False, _budget: Budget | None = None) -> dict:
    with Batch(path, _budget if _budget is not None else Budget(timeout_ms)) as batch:
        require(batch.id == expected_id, 'batch identity differs')
        out = batch.audit()
        require(type(allocation) is bool and not (allocation and step_name is not None),
                'allocation inspection and child receipt selection are separate bounded views')
        if allocation:
            require(batch.handoff is not None, 'legacy exact-item batch has no retained allocation')
            out['allocation'] = batch.handoff.json()
            out['handoff_digest'] = batch.handoff.digest
        if step_name is not None:
            model.label(step_name)
            step = next((s for s in batch.plan.steps if s.name == step_name), None)
            require(step is not None, 'step outside batch')
            journal = batch.effects.get(batch.key(step))
            out['step'] = journal.state.view()
            record = journal.state.terminal
            if record and record.phase == 'placed':
                out['receipt'] = {'canonical_record_hex': record.raw.hex()}
        batch.check()
        encoded('inspect', out)
        return out


def recover(path: str, expected_id: str, step_name: str, cancel: bool = False, timeout_ms: int = 10000,
            *, _budget: Budget | None = None) -> dict:
    with Batch(path, _budget if _budget is not None else Budget(timeout_ms), True) as batch:
        require(batch.id == expected_id, 'batch identity differs')
        step = next((s for s in batch.plan.steps if s.name == step_name), None)
        require(step is not None, 'step outside batch')
        journal = batch.effects.get(batch.key(step))
        batch.register(step)  # Adopt a crash-before-registration intent, never dispatch it.
        authority = None if journal.state.terminal is not None else Authority.load()
        if batch.room_handoff is not None and authority is not None:
            batch.budget.publication_authority = authority
        batch.check()
        outcome = placement.recover(journal, authority, cancel)
        out = batch.audit()
        out.update(native_contacted=outcome['native_contacted'], recovered_step=step.name,
                   native_query_status=outcome.get('effect_status'))
        encoded('cancel' if cancel else 'query', out)
        if batch.room_handoff is not None:
            if authority is not None:
                authority.guard('QueryPlacement')
            batch.budget.remaining()
            batch.check()
        return out


def stop(path: str, expected_id: str, timeout_ms: int = 10000, *, _budget: Budget | None = None) -> dict:
    with Batch(path, _budget if _budget is not None else Budget(timeout_ms), True) as batch:
        require(batch.id == expected_id, 'batch identity differs')
        out = batch.stop()
        encoded('stop', out)
        return out


def export_room_plan(path: str, expected_id: str, timeout_ms: int = 10000,
                     *, _budget: Budget | None = None) -> bytes:
    """Offline original goal recovery; never assemble a smaller successful subset."""
    with Batch(path, _budget if _budget is not None else Budget(timeout_ms)) as batch:
        require(batch.id == expected_id and batch.room_handoff is not None,
                'original room plan is absent or batch identity differs')
        out = batch.audit()
        encoded('inspect', out)  # Reserve the complete active-work disclosure too.
        raw = batch.room_handoff.room_plan.encode()
        batch.check()
        batch.budget.remaining()
        return raw


def check_definition_depth(raw: bytes, budget: Budget, *, terrain: bool = False) -> None:
    maximum = MAX_TERRAIN_DEFINITION if terrain else MAX_ROOM_DEFINITION
    require(type(raw) is bytes and 1 <= len(raw) <= maximum,
            'batch definition byte bound exceeded')
    depth, quoted, escaped = 0, False, False
    for offset, byte in enumerate(raw):
        if offset % 256 == 0:
            budget.remaining()
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
            require(depth <= (18 if terrain else 16), 'batch definition nesting bound exceeded')
        elif byte in (93, 125):
            depth -= 1
    budget.remaining()


def read_input(path: str, maximum: int) -> bytes:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        # Input is operator data, not persistent custody. Refuse special files.
        import stat
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= maximum, 'invalid plan input')
        raw = os.read(fd, before.st_size + 1)
        require(len(raw) == before.st_size and storage.stamp(before) == storage.stamp(os.fstat(fd))
                == storage.stamp(os.stat(path, follow_symlinks=False)), 'plan input changed')
        return raw
    finally:
        os.close(fd)


def read_plan(path: str) -> model.FurniturePlan:
    return model.FurniturePlan.decode(read_input(path, model.MAX_BYTES))


def read_handoff(path: str) -> Handoff:
    return Handoff.decode(read_input(path, MAX_HANDOFF_BYTES))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    for operation in ('init', 'inspect', 'review', 'advance', 'query', 'cancel', 'stop'):
        command = sub.add_parser(operation)
        command.add_argument('--directory', required=True)
        command.add_argument('--timeout-ms', type=int, default=10000)
        if operation == 'init':
            choice = command.add_mutually_exclusive_group(required=True)
            choice.add_argument('--plan')
            choice.add_argument('--handoff')
            choice.add_argument('--room-handoff')
            choice.add_argument('--terrain-handoff')
            command.add_argument('--world-folder')
            command.add_argument('--site', type=int)
        else:
            command.add_argument('--batch-id', required=True)
        if operation in ('inspect', 'query', 'cancel'):
            command.add_argument('--step', required=operation != 'inspect')
        if operation == 'inspect':
            command.add_argument('--allocation', action='store_true')
            command.add_argument('--emit', choices=('inventory', 'room-plan'), default='inventory')
        if operation == 'advance':
            command.add_argument('--expected-plan', required=True)
            command.add_argument('--confirm-review', required=True)
    args = parser.parse_args(argv)
    try:
        budget = Budget(args.timeout_ms)
        if args.operation == 'init':
            if args.terrain_handoff is not None:
                require(args.world_folder is None and args.site is None,
                        'terrain handoff supplies original fortress; overrides are not accepted')
                authority = Authority.load()
                raw = read_input(args.terrain_handoff, MAX_TERRAIN_HANDOFF_BYTES)
                terrain = TerrainFurnitureHandoff.decode(raw, ReadBudget(budget).checkpoint)
                authority.guard('QueryPlacement')
                h = terrain.room.allocation
                out = initialize(args.directory, h.plan(), h.request.folder, h.request.site,
                                 terrain_handoff=terrain, _budget=budget)
            elif args.room_handoff is not None:
                require(args.world_folder is None and args.site is None,
                        'room handoff supplies original fortress; overrides are not accepted')
                authority = Authority.load()
                raw = read_input(args.room_handoff, MAX_ROOM_HANDOFF_BYTES)
                room = RoomFurnitureHandoff.decode(raw, budget.remaining)
                authority.guard('QueryPlacement')
                h = room.allocation
                out = initialize(args.directory, h.plan(), h.request.folder, h.request.site,
                                 room_handoff=room, _budget=budget)
            else:
                require(args.world_folder is not None and args.site is not None,
                        'legacy batch imports require explicit fortress selectors')
                handoff = read_handoff(args.handoff) if args.handoff is not None else None
                plan = handoff.plan() if handoff is not None else read_plan(args.plan)
                out = initialize(args.directory, plan, args.world_folder, args.site,
                                 handoff=handoff, _budget=budget)
        elif args.operation == 'advance':
            out = advance(args.directory, args.batch_id, args.expected_plan, args.confirm_review, _budget=budget)
        elif args.operation == 'inspect':
            if args.emit == 'room-plan':
                require(args.step is None and not args.allocation, 'original-room export cannot select a child or allocation')
                output = export_room_plan(args.directory, args.batch_id, _budget=budget)
                budget.remaining()
                return write_output(output)
            out = inspect(args.directory, args.batch_id, args.step, allocation=args.allocation, _budget=budget)
        elif args.operation in ('query', 'cancel'):
            out = recover(args.directory, args.batch_id, args.step, args.operation == 'cancel', _budget=budget)
        else:
            out = {'review': review, 'stop': stop}[args.operation](args.directory, args.batch_id, _budget=budget)
        output = encoded(args.operation, out)
        authority = getattr(budget, 'publication_authority', None)
        if authority is not None:
            authority.guard('QueryPlacement')  # Still the original read credential, never a reloaded replacement.
        budget.remaining()
        code = 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        out = {'inventory_verified': False, 'effect_status': 'unknown', 'retry_permitted': False,
               'construction_completion_proven': False,
               'error': 'Request, source, custody, confirmation or budget refused. Preserve the batch and recover original keys; do not repeat placement.'}
        output = encoded(args.operation, out, False)
        code = 2
    terrain = getattr(budget, 'publication_terrain', None) if code == 0 else None
    if terrain is not None:
        attempted = False
        try:
            # Final CLI serialization may invoke user I/O hooks. Reopen the exact
            # original under the SAME budget and hold it through the single write.
            with terrain.origin.open(terrain.room.room_plan, budget):
                if authority is not None:
                    authority.guard('QueryPlacement')
                budget.remaining()
                attempted = True
                return code if write_output(output) == 0 else 2
        except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
            if not attempted:
                write_output(encoded(args.operation, {'inventory_verified': False, 'effect_status': 'unknown',
                    'retry_permitted': False, 'construction_completion_proven': False,
                    'error': 'Final terrain custody or authority refused. Recover original keys; do not repeat placement.'}, False))
            return 2
    return code if write_output(output) == 0 else 2


def write_output(raw: bytes) -> int:
    # No second object, native retry or journal repair after short output I/O.
    try:
        if sys.stdout.buffer.write(raw) != len(raw):
            return 2
        sys.stdout.buffer.flush()
    except (OSError, ValueError):
        return 2
    return 0



if __name__ == '__main__':
    raise SystemExit(main())
