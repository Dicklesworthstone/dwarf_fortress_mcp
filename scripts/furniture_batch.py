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

import furniture_plan as model
import build_placement_client as placement
import build_placement_store as storage
from build_placement_rpc import Authority, Budget, Client, Reply, endpoint
from build_placement_wire import Plan, Selection, canonical, exact_hex, integer, require, text_bytes, MAX_TICK

SCHEMA = 'dfmcp.furniture-batch/1'
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


class File:
    """One bounded descriptor-pinned batch metadata file, never a native journal."""
    def __init__(self, root: int, name: str, budget: Budget, writable: bool, initial: bytes | None = None):
        self.root, self.name, self.budget = root, name, budget
        self.fd = None
        try:
            flags = os.O_RDWR | os.O_APPEND if writable else os.O_RDONLY
            if initial is not None:
                require(writable and 0 < len(initial) <= MAX_FILE, 'invalid batch publication')
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
        require(identity(self.fd) == self.identity and 0 < before.st_size <= MAX_FILE, 'invalid batch file identity or extent')
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
        self.lock = root_lock(path, budget)
        self.fd = self.lock.__enter__()
        try:
            self.root_identity = identity(self.fd)
            found = names(self.fd)
            require({'batch.json', 'steps.jsonl', 'effects'} <= found
                    and found <= {'batch.json', 'steps.jsonl', 'effects', 'stop.json'}, 'incomplete or foreign batch inventory')
            for name in sorted(found - {'effects'}):
                self.files[name] = File(self.fd, name, budget, writable and name == 'steps.jsonl')
            value = unseal(self.files['batch.json'].raw)
            storage.exact_object(value, {'schema', 'nonce', 'plan', 'source', 'endpoint', 'folder', 'site',
                                        'dimensions', 'first_tick', 'root_identity', 'effects_identity'})
            require(value['schema'] == SCHEMA, 'unsupported batch format')
            exact_hex(value['nonce'], 24)
            self.plan = model.FurniturePlan.from_json(value['plan'])
            self.source = storage.manifest_from(value['source'])
            self.address = endpoint(value['endpoint'])
            text_bytes(value['folder'], 512)
            integer(value['site'], 0, 2147483647)
            integer(value['first_tick'], 0, MAX_TICK)
            self.plan.check_dimensions(value['dimensions'])
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

    def bind(self, plan: Plan, manifest) -> None:
        self.bind_capture(plan.before, manifest)

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
        if self.effects is not None:
            self.effects.close()
            self.effects = None
        for file in self.files.values():
            file.close()
        self.files.clear()
        if self.fd is not None:
            self.lock.__exit__(None, None, None)
            self.fd = None

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


def encoded(operation: str, out: dict, ok: bool = True) -> bytes:
    raw = canonical(packet(operation, out, ok)) + b'\n'
    require(len(raw) <= MAX_OUTPUT, 'complete furnishing result exceeds 64 KiB')
    return raw


def initialize(path: str, plan: model.FurniturePlan, folder: str, site: int, timeout_ms: int = 10000) -> dict:
    text_bytes(folder, 512)
    integer(site, 0, 2147483647)
    budget = Budget(timeout_ms)
    with root_lock(path, budget) as root:
        require(not names(root), 'initialization requires an existing empty private directory')
        authority = Authority.load()
        with Client(authority, budget, selection(plan.ordered[0])) as client:
            reply = client.observe()
        capture = reply.capture
        require(capture.folder == folder and capture.site == site, 'initial observation is another fortress')
        plan.check_dimensions(capture.dimensions)
        os.mkdir('effects', 0o700, dir_fd=root)
        effects = storage.open_directory(str(Path(path) / 'effects'))
        try:
            value = {'schema': SCHEMA, 'nonce': secrets.token_hex(24), 'plan': plan.json(),
                'source': reply.manifest.view(), 'endpoint': f'{authority.address[0]}:{authority.address[1]}',
                'folder': folder, 'site': site, 'dimensions': list(capture.dimensions), 'first_tick': capture.tick,
                'root_identity': identity(root), 'effects_identity': identity(effects)}
        finally:
            os.close(effects)
        raw = seal(value)
        require(len(raw) <= MAX_FILE, 'batch manifest too large')
        published = []
        try:
            for name, data in [('batch.json', raw), ('steps.jsonl', HEADER)]:
                published.append(File(root, name, budget, True, data))
            for file in published:
                file.check()
        finally:
            for file in published:
                file.close()
    with Batch(path, budget) as batch:
        out = batch.audit()
        out['native_contacted'] = True
        encoded('init', out)
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


def review(path: str, expected_id: str, timeout_ms: int = 10000) -> dict:
    with Batch(path, Budget(timeout_ms)) as batch:
        step, out = select_next(batch, expected_id)
        authority = Authority.load()
        require(authority.address == batch.address, 'operator endpoint differs from original batch')
        with Client(authority, batch.budget, selection(step)) as client:
            reply = client.observe()
        # Even an ineligible capture is bound before its diagnostics are exposed.
        require(reply.manifest == batch.source and reply.capture.identity == (
            batch.source.generation, batch.value['site'], tuple(batch.value['dimensions']), batch.value['folder'])
            and reply.capture.tick >= batch.value['first_tick'], 'review source differs from batch')
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
        return out


def advance(path: str, expected_id: str, expected_plan: str, confirmation: str, timeout_ms: int = 10000) -> dict:
    exact_hex(expected_plan, 32)
    exact_hex(confirmation, 32)
    with Batch(path, Budget(timeout_ms), True) as batch:
        step, initial = select_next(batch, expected_id)
        require(len(encoded('advance', initial)) + 32768 < MAX_OUTPUT, 'batch outcome reservation exceeds bound')
        authority = Authority.load(True)
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
        return out


def inspect(path: str, expected_id: str, step_name: str | None = None, timeout_ms: int = 10000) -> dict:
    with Batch(path, Budget(timeout_ms)) as batch:
        require(batch.id == expected_id, 'batch identity differs')
        out = batch.audit()
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


def recover(path: str, expected_id: str, step_name: str, cancel: bool = False, timeout_ms: int = 10000) -> dict:
    with Batch(path, Budget(timeout_ms), True) as batch:
        require(batch.id == expected_id, 'batch identity differs')
        step = next((s for s in batch.plan.steps if s.name == step_name), None)
        require(step is not None, 'step outside batch')
        journal = batch.effects.get(batch.key(step))
        batch.register(step)  # Adopt a crash-before-registration intent, never dispatch it.
        authority = None if journal.state.terminal is not None else Authority.load()
        batch.check()
        outcome = placement.recover(journal, authority, cancel)
        out = batch.audit()
        out.update(native_contacted=outcome['native_contacted'], recovered_step=step.name,
                   native_query_status=outcome.get('effect_status'))
        encoded('cancel' if cancel else 'query', out)
        return out


def stop(path: str, expected_id: str, timeout_ms: int = 10000) -> dict:
    with Batch(path, Budget(timeout_ms), True) as batch:
        require(batch.id == expected_id, 'batch identity differs')
        out = batch.stop()
        encoded('stop', out)
        return out


def read_plan(path: str) -> model.FurniturePlan:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        # Input is operator data, not persistent custody. Refuse special files.
        import stat
        require(stat.S_ISREG(before.st_mode) and 1 <= before.st_size <= model.MAX_BYTES, 'invalid plan input')
        raw = os.read(fd, before.st_size + 1)
        require(len(raw) == before.st_size and storage.stamp(before) == storage.stamp(os.fstat(fd))
                == storage.stamp(os.stat(path, follow_symlinks=False)), 'plan input changed')
        return model.FurniturePlan.decode(raw)
    finally:
        os.close(fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='operation', required=True)
    for operation in ('init', 'inspect', 'review', 'advance', 'query', 'cancel', 'stop'):
        command = sub.add_parser(operation)
        command.add_argument('--directory', required=True)
        command.add_argument('--timeout-ms', type=int, default=10000)
        if operation == 'init':
            command.add_argument('--plan', required=True)
            command.add_argument('--world-folder', required=True)
            command.add_argument('--site', required=True, type=int)
        else:
            command.add_argument('--batch-id', required=True)
        if operation in ('inspect', 'query', 'cancel'):
            command.add_argument('--step', required=operation != 'inspect')
        if operation == 'advance':
            command.add_argument('--expected-plan', required=True)
            command.add_argument('--confirm-review', required=True)
    args = parser.parse_args(argv)
    try:
        if args.operation == 'init':
            out = initialize(args.directory, read_plan(args.plan), args.world_folder, args.site, args.timeout_ms)
        elif args.operation == 'advance':
            out = advance(args.directory, args.batch_id, args.expected_plan, args.confirm_review, args.timeout_ms)
        elif args.operation == 'inspect':
            out = inspect(args.directory, args.batch_id, args.step, args.timeout_ms)
        elif args.operation in ('query', 'cancel'):
            out = recover(args.directory, args.batch_id, args.step, args.operation == 'cancel', args.timeout_ms)
        else:
            out = {'review': review, 'stop': stop}[args.operation](args.directory, args.batch_id, args.timeout_ms)
        print(encoded(args.operation, out).decode('ascii'), end='')
        return 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        out = {'inventory_verified': False, 'effect_status': 'unknown', 'retry_permitted': False,
               'construction_completion_proven': False,
               'error': 'Request, source, custody, confirmation or budget refused. Preserve the batch and recover original keys; do not repeat placement.'}
        print(encoded(args.operation, out, False).decode('ascii'), end='')
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
