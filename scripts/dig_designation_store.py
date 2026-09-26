"""Directory-scoped custody for the existing dig developer workflow.

No protocol implementation, alternate setter or MCP entry point lives here. The
caller supplies the existing client's codec and one bounded native connection.
"""
from __future__ import annotations

from contextlib import contextmanager, nullcontext
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time

REGISTRY = '.dfmcp-dig-registry.jsonl'
HEADER = b'{"format":"dfmcp.dig-registry/1"}\n'
MAX_RECORDS = 128
MAX_REGISTRY = 131072
MAX_ENTRIES = 2 * MAX_RECORDS + 1


def fingerprint(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class Store:
    def __init__(self, api, root: Path, parent: int, writable: bool, budget):
        self.api, self.root, self.parent = api, root, parent
        self.writable, self.budget = writable, budget
        self.identity = (os.fstat(parent).st_dev, os.fstat(parent).st_ino)
        self.fd = None
        self.raw = HEADER
        self.entries = []
        self.pinned = None

    def name_ok(self, name):
        self.api.key_bytes(name)
        self.api.require(name not in ('.', '..') and not name.startswith('.dfmcp-'),
                         'reserved or invalid intent filename')

    def verify(self):
        self.budget()
        actual, opened = os.stat(self.root, follow_symlinks=False), os.fstat(self.parent)
        self.api.require(self.root.resolve(strict=True) == self.root
            and (actual.st_dev, actual.st_ino) == (opened.st_dev, opened.st_ino) == self.identity
            and stat.S_ISDIR(actual.st_mode) and stat.S_IMODE(actual.st_mode) == 0o700
            and actual.st_uid == opened.st_uid and actual.st_uid in (0, os.geteuid()),
            'dig store directory custody changed')
        if self.pinned is not None:
            self.pinned.verify()
        else:
            try:
                os.stat(REGISTRY, dir_fd=self.parent, follow_symlinks=False)
            except FileNotFoundError:
                return
            self.api.require(False, 'registry appeared without custody')

    def open_registry(self):
        flags = (os.O_RDWR | os.O_APPEND if self.writable else os.O_RDONLY)
        try:
            self.fd = os.open(REGISTRY, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                              dir_fd=self.parent)
        except FileNotFoundError:
            self.verify()
            return
        before = os.fstat(self.fd)
        self.api.require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
            and before.st_nlink == 1 and before.st_uid == os.fstat(self.parent).st_uid
            and len(HEADER) <= before.st_size <= MAX_REGISTRY, 'invalid or incomplete dig registry')
        data = bytearray()
        while len(data) <= before.st_size:
            self.budget()
            part = os.read(self.fd, before.st_size + 1 - len(data))
            if not part:
                break
            data += part
        self.raw = bytes(data)
        after = os.fstat(self.fd)
        self.api.require((before.st_mtime_ns, before.st_ctime_ns, before.st_size)
            == (after.st_mtime_ns, after.st_ctime_ns, after.st_size)
            and len(data) == before.st_size, 'registry changed during read')
        self.entries = self.decode(self.raw)
        self.pinned = self.api.Capsule(self.root / REGISTRY, self.fd, self.parent, self.raw, {})
        self.verify()

    def decode(self, raw):
        self.api.require(raw.startswith(HEADER) and raw.endswith(b'\n') and len(raw) <= MAX_REGISTRY,
                         'corrupt registry; no repair performed')
        lines = raw[len(HEADER):].splitlines(keepends=True)
        self.api.require(len(lines) <= MAX_RECORDS, 'registry record bound exhausted')
        previous = fingerprint(HEADER)
        names, identities, entries = set(), set(), []
        for line in lines:
            self.budget()
            loaded = json.loads(line, object_pairs_hook=self.api.unique_object)
            self.api.require(isinstance(loaded, dict) and set(loaded) == {'entry', 'sha256'}, 'invalid registry envelope')
            entry = loaded['entry']
            self.api.require(isinstance(entry, dict) and set(entry) == {'name', 'intent_sha256', 'previous'},
                             'invalid registry entry')
            self.name_ok(entry['name']); self.api.exact_hex(entry['intent_sha256'], 32)
            digest = fingerprint(self.api.canonical(entry))
            self.api.require(entry['previous'] == previous and loaded['sha256'] == digest
                and line == self.api.canonical(loaded) + b'\n' and entry['name'] not in names
                and entry['intent_sha256'] not in identities, 'registry chain, identity or encoding mismatch')
            names.add(entry['name']); identities.add(entry['intent_sha256'])
            entries.append(entry); previous = digest
        return entries

    def initialize(self):
        self.api.require(self.writable, 'read-only registry access')
        self.verify()
        if self.fd is not None:
            return
        self.fd = os.open(REGISTRY, os.O_RDWR | os.O_APPEND | os.O_CREAT | os.O_EXCL
                          | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, 0o600, dir_fd=self.parent)
        self.write_all(HEADER)
        self.pinned = self.api.Capsule(self.root / REGISTRY, self.fd, self.parent, HEADER, {})
        self.verify(); os.fsync(self.fd); os.fsync(self.parent); self.verify()

    def write_all(self, raw):
        view = memoryview(raw)
        while view:
            self.budget()
            count = os.write(self.fd, view)
            self.api.require(count > 0, 'short registry append; no repair performed')
            view = view[count:]

    def names(self):
        self.verify()
        out = []
        with os.scandir(self.parent) as entries:
            for entry in entries:
                self.budget()
                self.api.require(len(out) < MAX_ENTRIES, 'store discovery bound exhausted')
                out.append(entry.name)
        self.verify()
        return sorted(out)

    def audit(self, current=None):
        self.verify()
        before = os.fstat(self.parent)
        names = self.names()
        indexed = {entry['name']: entry['intent_sha256'] for entry in self.entries}
        records, expected_receipts, seen_keys, seen_hashes = [], set(), set(), set()
        for name in names:
            self.budget()
            if name == REGISTRY or re.fullmatch(r'\.dfmcp-dig-terminal-[0-9a-f]{64}\.json', name):
                continue
            self.name_ok(name)
            self.api.require(len(records) < MAX_RECORDS, 'store intent bound exhausted')
            selected = nullcontext(current) if current is not None and current.path == self.root / name else self.api.capsule(self.root / name)
            with selected as owner:
                owner.verify()
                self.api.require(owner.parent_identity == self.identity, 'intent escaped pinned store')
                identity = fingerprint(owner.raw)
                self.api.require(name not in indexed or indexed[name] == identity, 'registered intent changed')
                self.api.require(identity not in seen_hashes and owner.intent['key'] not in seen_keys,
                                 'duplicate intent identity or key in store')
                seen_hashes.add(identity); seen_keys.add(owner.intent['key'])
                receipt = self.api.terminal_receipt(owner)
                expected_receipts.add(self.api.terminal_name(owner))
                records.append({'name': name, 'key': owner.intent['key'], 'intent_sha256': identity,
                    'plan_digest': owner.intent['plan_digest'], 'region': owner.intent['region'],
                    'manifest': owner.intent['manifest'], 'endpoint': owner.intent['endpoint'],
                    'registered': name in indexed, 'effect_status': receipt['effect']['state'] if receipt else 'unknown',
                    'terminal_receipt_sha256': receipt['terminal_receipt_sha256'] if receipt else None})
                owner.verify()
        after = os.fstat(self.parent)
        self.api.require((before.st_mtime_ns, before.st_ctime_ns) == (after.st_mtime_ns, after.st_ctime_ns),
                         'store directory changed during audit')
        found = {record['name'] for record in records}
        self.api.require(set(indexed) <= found, 'registered intent missing; never forget unresolved work')
        self.api.require(set(names) <= found | expected_receipts | {REGISTRY}, 'orphan or unexpected store entry')
        self.verify()
        return records

    def register(self, owner):
        self.api.require(self.writable and len(self.entries) < MAX_RECORDS, 'registry capacity exhausted')
        self.name_ok(owner.path.name); owner.verify()
        self.api.require(owner.path.parent == self.root and owner.parent_identity == self.identity,
                         'intent does not belong to this store')
        identity = fingerprint(owner.raw)
        self.api.require(all(e['name'] != owner.path.name and e['intent_sha256'] != identity for e in self.entries),
                         'intent already registered')
        self.initialize()
        previous = fingerprint(HEADER) if not self.entries else json.loads(self.raw.splitlines()[-1])['sha256']
        entry = {'name': owner.path.name, 'intent_sha256': identity, 'previous': previous}
        line = self.api.canonical({'entry': entry, 'sha256': fingerprint(self.api.canonical(entry))}) + b'\n'
        self.api.require(len(self.raw) + len(line) <= MAX_REGISTRY, 'registry byte bound exhausted')
        self.verify(); owner.verify()
        self.write_all(line)
        self.raw += line
        self.entries.append(entry); self.pinned.raw = self.raw
        self.verify(); owner.verify(); os.fsync(self.fd); os.fsync(self.parent)
        self.verify(); owner.verify()

    def ready(self, name, key):
        self.name_ok(name); self.api.key_bytes(key)
        records = self.audit()
        # Conservatively adopt valid legacy/orphan intents before admitting new
        # work. A crash cannot turn an unregistered intent into permission to retry.
        for record in records:
            if not record['registered']:
                with self.api.capsule(self.root / record['name']) as owner:
                    self.api.require(fingerprint(owner.raw) == record['intent_sha256'], 'legacy intent changed')
                    self.register(owner)
        self.api.require(len(records) < MAX_RECORDS, 'store capacity exhausted; no automatic eviction')
        self.api.require(all(r['effect_status'] != 'unknown' for r in records),
                         'unresolved dig intent blocks new work; query or cancel the original record')
        self.api.require(all(r['name'] != name and r['key'] != key for r in records),
                         'intent filename or key already used; do not replay')
        self.verify()

    def dispatch_check(self, owner):
        records = self.audit(owner)
        self.api.require(all(r['registered'] for r in records)
            and sum(r['intent_sha256'] == fingerprint(owner.raw) for r in records) == 1
            and all(r['effect_status'] != 'unknown' or r['intent_sha256'] == fingerprint(owner.raw) for r in records),
            'store no longer grants this sole pending dispatch')
        self.verify(); owner.verify()


@contextmanager
def open_store(api, root: Path, writable=False, budget=lambda: None):
    api.require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'dig store requires POSIX custody')
    import fcntl
    api.require(root.is_absolute() and '..' not in root.parts and 1 <= len(str(root)) <= 4096,
                'store directory must be normalized and absolute')
    parent = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    store = None
    try:
        for part in root.parts[1:]:
            budget()
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=parent)
            os.close(parent); parent = child
        fcntl.flock(parent, fcntl.LOCK_EX | fcntl.LOCK_NB)
        store = Store(api, root, parent, writable, budget)
        # Directory checks cannot assert registry absence before opening it.
        actual = os.fstat(parent)
        api.require(stat.S_IMODE(actual.st_mode) == 0o700 and actual.st_uid in (0, os.geteuid()),
                    'store must be an owned exact-mode 0700 directory')
        store.open_registry()
        yield store
    finally:
        if store is not None and store.fd is not None:
            os.close(store.fd)
        os.close(parent)


def start_designation(api, client, path, key, selected, allow_hidden, expected_witness, confirmed_plan, *, guard=None):
    # Optional enclosing-workflow validation runs UNDER the existing store lock.
    # It can only narrow admission; ordinary capsule callers retain the same path.
    api.key_bytes(key); api.region(selected); api.flag(allow_hidden)
    witness = api.exact_hex(expected_witness, 32); confirmed = api.exact_hex(confirmed_plan, 32)
    api.require(confirmed == api.plan_for(selected, allow_hidden, witness), 'confirmation differs from requested sealed plan')
    with open_store(api, path.parent, True, client.remaining) as store:
        store.ready(path.name, key)
        if guard is not None:
            guard(store, None)
        observed = client.observe(selected)
        api.require(hashlib.sha256(observed['raw']).digest() == witness, 'terrain changed since observation; no intent dispatched')
        intent = api.build_intent(client.address, key, selected, allow_hidden, observed['raw'], observed['manifest'])
        with api.capsule(path, intent) as owner:
            store.register(owner)
            store.dispatch_check(owner)
            if guard is not None:
                guard(store, owner)
            client.remaining()
            prepared = client.prepare(intent)
            if prepared['replayed']:
                result = api.finish_recovery(owner, prepared, False, True)
            else:
                store.dispatch_check(owner)
                if guard is not None:
                    guard(store, owner)
                client.remaining()
                result = api.finish_recovery(owner, client.commit(intent, prepared), True)
            store.verify(); owner.verify()
            if guard is not None:
                guard(store, owner)
            return result


def records_result(api, root, limit=8, continuation=None, timeout_ms=10000):
    api.integer(limit, 1, 8); api.integer(timeout_ms, 1, 60000)
    deadline = time.monotonic() + timeout_ms / 1000
    def budget():
        api.require(time.monotonic() < deadline, 'offline store inspection budget exhausted')
    with open_store(api, root, budget=budget) as store:
        records = store.audit()
        head = fingerprint(api.canonical({'directory': str(root), 'identity': store.identity,
            'registry': fingerprint(store.raw) if store.fd is not None else None, 'records': records}))
        offset = 0
        if continuation is not None:
            raw = api.exact_hex(continuation, 1, 256)
            cursor = json.loads(raw, object_pairs_hook=api.unique_object)
            api.require(isinstance(cursor, dict) and set(cursor) == {'head', 'offset', 'limit'}
                and api.canonical(cursor) == raw and cursor['head'] == head and cursor['limit'] == limit,
                'continuation does not match exact store snapshot and query')
            api.integer(cursor['limit'], 1, 8)
            offset = api.integer(cursor['offset'], 1, len(records))
            api.require(offset < len(records) and offset % limit == 0, 'invalid continuation offset')
        end = min(offset + limit, len(records))
        token = api.canonical({'head': head, 'offset': end, 'limit': limit}).hex() if end < len(records) else None
        result = {'ok': True, 'profile': 'dig/1.16', 'directory': str(root), 'snapshot': head,
            'records': records[offset:end], 'total_records': len(records), 'continuation': token,
            'unresolved_records': sum(r['effect_status'] == 'unknown' for r in records),
            'registry_present': store.fd is not None, 'native_calls': 0,
            'scope': 'private_directory_only', 'global_fence': False, 'mutation_authority_granted': False,
            'retry_commit_permitted': False, 'excavation_completion_proven': False}
        api.require(len(api.canonical(result)) <= api.MAX_OUTPUT, 'complete store response exceeds bound')
        store.verify()
        return result
