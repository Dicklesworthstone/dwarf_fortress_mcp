#!/usr/bin/env python3
"""POSIX developer control/recovery for order-run/1.14, not an MCP or admitted runner.

One private append-only journal owns sealed intents, dispatch markers and native
receipts. There is no reconnect, implicit commit retry, repair or game save.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import sys

import order_run_wire as w

MAX_BYTES, MAX_EVENTS, MAX_INTENTS, MAX_LINE = 2 * 1024 * 1024, 4096, 256, 8192
STATES = ('intent', 'prepared', 'dispatch_started', 'tracking', 'cancel_requested', 'terminal', 'cancelled_before_dispatch')


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def sealed(value):
    data = canonical(value)
    return canonical({'value': value, 'sha256': hashlib.sha256(data).hexdigest()}) + b'\n'


def unseal(data):
    w.require(1 <= len(data) <= MAX_LINE, 'journal line exceeds bound')
    envelope = json.loads(data)
    w.require(type(envelope) is dict and set(envelope) == {'value', 'sha256'} and sealed(envelope['value']) == data,
              'noncanonical or corrupt journal line')
    return envelope['value'], envelope['sha256']


def binding(endpoint, manifest, capture):
    w.address(endpoint)
    return dict(endpoint=endpoint, generation=manifest['generation'], df_version=manifest['df_version'],
                dfhack_version=manifest['dfhack_version'], folder=capture['folder'], site=capture['site'])


def validate_binding(value):
    w.require(type(value) is dict and set(value) == {'endpoint', 'generation', 'df_version', 'dfhack_version', 'folder', 'site'},
              'invalid journal binding')
    w.address(value['endpoint']); w.integer(value['generation'], 1, 2**64 - 2)
    w.integer(value['site'], 0, 2**31 - 1); w.text(value['folder'], 512)
    w.text(value['df_version']); w.text(value['dfhack_version'])


def native(entry):
    return w.record(w.unhex(entry['native'], 214, 1425)) if entry['native'] is not None else None


def unresolved(entry):
    return entry['state'] not in ('terminal', 'cancelled_before_dispatch') or (
        entry['state'] == 'terminal' and native(entry)['phase'] == 'source_lost')


def transition(previous, entry, source):
    w.require(type(entry) is dict and set(entry) == {'key', 'plan', 'state', 'native'}, 'invalid journal event fields')
    w.key(entry['key']); w.require(entry['state'] in STATES, 'unknown coordinator state')
    goal = w.plan(w.unhex(entry['plan'], 93, 604)); before = goal['before']
    w.require(all(before[k] == source[k] for k in ('generation', 'folder', 'site')), 'plan belongs to a different source')
    proof = native(entry); state = entry['state']
    if proof:
        w.require(proof['key'] == entry['key'] and proof['plan_hex'] == entry['plan'], 'journal receipt differs from sealed intent')
    if previous is None:
        w.require(state == 'intent' and proof is None, 'first event must retain only intent')
        return
    w.require(entry['plan'] == previous['plan'] and previous['state'] not in ('terminal', 'cancelled_before_dispatch'),
              'operation was rewritten or terminal evidence changed')
    allowed = {
        'intent': ('prepared', 'tracking', 'terminal', 'cancelled_before_dispatch'),
        'prepared': ('dispatch_started', 'tracking', 'terminal', 'cancelled_before_dispatch'),
        'dispatch_started': ('tracking', 'terminal', 'cancel_requested'),
        'tracking': ('tracking', 'terminal', 'cancel_requested'),
        'cancel_requested': ('tracking', 'terminal', 'cancel_requested'),
    }
    w.require(state in allowed[previous['state']], 'illegal coordinator transition')
    if state in ('dispatch_started', 'cancel_requested', 'cancelled_before_dispatch'):
        w.require(entry['native'] == previous['native'], 'local transition rewrote native evidence')
    else:
        w.require(proof is not None, 'receipt transition lacks native evidence')
        w.require((state == 'prepared' and proof['phase'] == 'prepared') or
                  (state == 'tracking' and proof['phase'] in ('prepared', 'running', 'stopping')) or
                  (state == 'terminal' and proof['phase'] in ('stopped', 'refused', 'source_lost')), 'native/coordinator states disagree')
        old = native(previous)
        if old:
            permitted = {'prepared': set(w.PHASES), 'running': {'running', 'stopping', 'stopped', 'source_lost'},
                         'stopping': {'stopping', 'stopped', 'source_lost'}}
            w.require(proof['phase'] in permitted.get(old['phase'], ()), 'native receipt regressed')
            if proof['phase'] == 'running' and old['observed_tick'] is not None:
                w.require(proof['observed_tick'] >= old['observed_tick'], 'running evidence clock regressed')
            if old['trigger'] != 'none':
                w.require(all(proof[k] == old[k] for k in ('trigger', 'sample', 'reported_stable_samples', 'counted_tick')),
                          'frozen trigger evidence changed')
    if state == 'dispatch_started':
        w.require(proof is not None and proof['phase'] == 'prepared', 'dispatch lacks preparation')


class Journal:
    def __init__(self, fd, parent, path, writable, data, header):
        self.fd, self.parent, self.path, self.writable = fd, parent, path, writable
        self.data, self.header, self.entries, self.events, self.fenced = data, header, {}, 0, False
        self.head = hashlib.sha256(canonical(header)).hexdigest()
        lines = data.splitlines(keepends=True)
        for line in lines[1:]:
            event, checksum = unseal(line)
            w.require(type(event) is dict and set(event) == {'sequence', 'previous', 'entry'}, 'invalid journal frame')
            self.events += 1
            w.require(type(event['sequence']) is int and event['sequence'] == self.events and event['previous'] == self.head,
                      'journal gap, fork or reordering')
            self.accept(event['entry']); self.head = checksum
        w.require(self.events <= MAX_EVENTS, 'journal transition limit exceeded')
        self.identity = (os.fstat(fd).st_dev, os.fstat(fd).st_ino)
        self.parent_identity = (os.fstat(parent).st_dev, os.fstat(parent).st_ino)

    def accept(self, entry):
        previous = self.entries.get(entry.get('key')) if type(entry) is dict else None
        transition(previous, entry, self.header['binding'])
        if previous is None:
            w.require(len(self.entries) < MAX_INTENTS and not any(unresolved(e) for e in self.entries.values()),
                      'new intent cannot bypass unfinished or source-lost work')
        self.entries[entry['key']] = entry

    def verify(self):
        w.require(not self.fenced, 'journal fenced; reopen existing evidence without retrying commit')
        current = os.fstat(self.fd); named = os.stat(self.path.name, dir_fd=self.parent, follow_symlinks=False)
        parent = os.fstat(self.parent)
        with parent_directory(self.path) as resolved:
            resolved_info = os.fstat(resolved)
            w.require((resolved_info.st_dev, resolved_info.st_ino) == self.parent_identity, 'named parent was replaced')
        w.require(stat.S_ISREG(current.st_mode) and stat.S_IMODE(current.st_mode) == 0o600
                  and current.st_uid in (0, os.geteuid()) and current.st_nlink == 1
                  and current.st_size == len(self.data) and (current.st_dev, current.st_ino) == self.identity
                  and (named.st_dev, named.st_ino) == self.identity and stat.S_IMODE(parent.st_mode) == 0o700,
                  'journal identity, extent or custody changed')
        os.lseek(self.fd, 0, os.SEEK_SET)
        read = bytearray()
        while len(read) < len(self.data):
            chunk = os.read(self.fd, len(self.data) - len(read)); w.require(chunk, 'journal truncated during verification'); read += chunk
        after = os.fstat(self.fd)
        w.require(bytes(read) == self.data and (after.st_mtime_ns, after.st_ctime_ns) == (current.st_mtime_ns, current.st_ctime_ns),
                  'journal bytes changed')

    def get(self, key):
        self.verify(); w.key(key); value = self.entries.get(key)
        w.require(value is not None, 'operation not recorded in this journal'); return value

    def append(self, entry):
        w.require(self.writable and not self.fenced, 'journal is read-only or fenced')
        self.verify()
        previous = self.entries.get(entry['key']); transition(previous, entry, self.header['binding'])
        if previous is None:
            w.require(len(self.entries) < MAX_INTENTS and not any(unresolved(e) for e in self.entries.values()),
                      'unfinished or source-lost work blocks new intents')
        if previous == entry:
            return
        reserve = 0 if entry['state'] in ('terminal', 'cancelled_before_dispatch') else 1 if entry['state'] == 'cancel_requested' else 2
        line = sealed(dict(sequence=self.events + 1, previous=self.head, entry=entry))
        w.require(len(line) <= MAX_LINE and self.events + 1 + reserve <= MAX_EVENTS
                  and len(self.data) + len(line) + reserve * MAX_LINE <= MAX_BYTES, 'journal capacity reserved for stop evidence')
        try:
            os.lseek(self.fd, len(self.data), os.SEEK_SET); write_all(self.fd, line); os.fsync(self.fd)
            # Do not acknowledge/publish an event whose named custody changed.
            original = self.data; self.data += line
            try:
                self.verify()
            except BaseException:
                self.data = original; raise
            _, self.head = unseal(line); self.events += 1; self.entries[entry['key']] = entry
        except BaseException:
            self.fenced = True; raise

    def retain(self, entry, reply):
        self.verify()
        source = self.header['binding']; manifest = reply['manifest']
        w.require(manifest['generation'] >= source['generation'] and all(manifest[k] == source[k]
                  for k in ('df_version', 'dfhack_version')), 'reply software/source differs from journal')
        if 'record_hex' not in reply:
            # Keep last verified evidence historical; absence cannot erase it or
            # turn a previous dispatch into a retryable preparation.
            return dict(native_record_absent=True, effect_status='unknown', retained=entry)
        proof = reply['record']; phase = proof['phase']
        state = 'terminal' if phase in ('stopped', 'refused', 'source_lost') else (
            'prepared' if entry['state'] == 'intent' and phase == 'prepared' else 'tracking')
        if entry['state'] == 'prepared' and phase == 'prepared' and entry['native'] == reply['record_hex']:
            return dict(native_record_absent=False, retained=entry)
        updated = dict(entry, state=state, native=reply['record_hex'])
        if updated != entry:
            self.append(updated)
        return dict(native_record_absent=False, retained=updated)


@contextmanager
def parent_directory(path):
    w.require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW') and path.is_absolute() and '..' not in path.parts
              and bool(path.name) and len(str(path)) <= 4096, 'normalized absolute POSIX journal path required')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd); fd = child
        info = os.fstat(fd)
        w.require(stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid in (0, os.geteuid()), 'private 0700 parent required')
        yield fd
    finally:
        os.close(fd)


def write_all(fd, data):
    remaining = memoryview(data)
    while remaining:
        count = os.write(fd, remaining); w.require(count > 0, 'short journal write'); remaining = remaining[count:]


@contextmanager
def open_journal(path, writable=False, create_binding=None):
    import fcntl
    w.require(create_binding is None or writable, 'read-only mode cannot initialize')
    if create_binding is not None:
        validate_binding(create_binding)
    with parent_directory(path) as parent:
        flags = (os.O_RDWR if writable else os.O_RDONLY) | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
        created = False
        try:
            fd = os.open(path.name, flags, dir_fd=parent)
        except FileNotFoundError:
            w.require(create_binding is not None, 'recovery requires an existing journal')
            fd = os.open(path.name, flags | os.O_CREAT | os.O_EXCL, 0o600, dir_fd=parent); created = True
        try:
            fcntl.flock(fd, (fcntl.LOCK_EX if writable else fcntl.LOCK_SH) | fcntl.LOCK_NB)
            info = os.fstat(fd)
            w.require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600
                      and info.st_uid in (0, os.geteuid()) and info.st_nlink == 1 and info.st_size <= MAX_BYTES,
                      'private single-link regular journal required')
            if created:
                header = dict(format='dfmcp.order-run-journal/1', binding=create_binding, identity=secrets.token_hex(32))
                data = sealed(header); write_all(fd, data); os.fsync(fd); os.fsync(parent)
            else:
                data = bytearray()
                while len(data) <= info.st_size:
                    part = os.read(fd, info.st_size + 1 - len(data))
                    if not part:
                        break
                    data += part
                w.require(len(data) == info.st_size and data, 'journal changed, truncated or empty')
                data = bytes(data); header, _ = unseal(data.splitlines(keepends=True)[0])
                w.require(type(header) is dict and set(header) == {'format', 'binding', 'identity'}
                          and header['format'] == 'dfmcp.order-run-journal/1', 'wrong journal profile')
                w.unhex(header['identity'], 32); validate_binding(header['binding'])
                if create_binding is not None:
                    w.require(header['binding'] == create_binding, 'journal belongs to another fortress/source')
            journal = Journal(fd, parent, path, writable, data, header); journal.verify()
            if writable:
                # Complete bytes after an uncertain previous sync are verified
                # and resynced. An incomplete tail is never repaired/truncated.
                os.fsync(fd); os.fsync(parent)
            yield journal
        finally:
            os.close(fd)


def summary(entry):
    proof = native(entry)
    return dict(key=entry['key'], state=entry['state'], plan_digest=w.plan_digest(bytes.fromhex(entry['plan'])).hex(),
                unresolved=unresolved(entry), native_evidence=proof, current_pause_unproved=True, goods_produced_proven=False)


def source_matches(journal, client):
    journal.verify(); source = journal.header['binding']; manifest = client.manifest
    w.require(client.endpoint == source['endpoint'] and manifest['generation'] >= source['generation']
              and all(manifest[k] == source[k] for k in ('df_version', 'dfhack_version')), 'connection differs from journal binding')


def prepare(client, path, operation_key, encoded_plan):
    goal = w.plan(encoded_plan); before = goal['before']; current = client.call('ObserveRun', order_id=before['order_id'])
    w.require(current['capture_hex'] == goal['capture_hex'], 'order/source changed before durable preparation')
    source = binding(client.endpoint, current['manifest'], before)
    with open_journal(path, writable=True, create_binding=source) as journal:
        existing = journal.entries.get(w.key(operation_key))
        if existing:
            w.require(existing['plan'] == encoded_plan.hex(), 'key already binds another plan'); return summary(existing)
        entry = dict(key=operation_key, plan=encoded_plan.hex(), state='intent', native=None)
        journal.append(entry)  # Sync complete intent before native preparation.
        response = client.call('PrepareRun', operation_key, encoded_plan)
        outcome = journal.retain(entry, response)
        return summary(outcome['retained'])


def operate(journal, client, operation, operation_key, confirmation=None):
    source_matches(journal, client); entry = journal.get(operation_key)
    encoded_plan = bytes.fromhex(entry['plan']); goal = w.plan(encoded_plan)
    if operation == 'commit':
        w.require(confirmation == w.plan_digest(encoded_plan).hex(), 'commit requires the exact reviewed plan digest')
        w.require(entry['state'] == 'prepared', 'commit is not replayable; query or cancel the recorded operation')
        observed = client.call('ObserveRun', order_id=goal['before']['order_id'])
        w.require(observed['capture_hex'] == goal['capture_hex'], 'commit requires the exact originally paused fortress and order')
        entry = dict(entry, state='dispatch_started'); journal.append(entry)
        response = client.call('CommitRun', operation_key, encoded_plan)  # Sole unpause attempt.
    elif operation == 'cancel':
        if entry['state'] in ('terminal', 'cancelled_before_dispatch'):
            return summary(entry)
        if entry['state'] in ('intent', 'prepared'):
            entry = dict(entry, state='cancelled_before_dispatch'); journal.append(entry); return summary(entry)
        entry = dict(entry, state='cancel_requested')
        if journal.entries[operation_key] != entry:
            journal.append(entry)
        response = client.call('CancelRun', operation_key, encoded_plan)
    else:
        w.require(operation == 'query', 'unsupported recovery operation')
        if entry['state'] in ('terminal', 'cancelled_before_dispatch'):
            return summary(entry)
        response = client.call('QueryRun', operation_key, encoded_plan)
    outcome = journal.retain(entry, response)
    return dict(summary(outcome['retained']), native_record_absent=outcome['native_record_absent'])


def environment(control):
    allowed = {'DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14', 'DFMCP_ORDER_RUN_TOKEN', 'DFMCP_ORDER_RUN_ENDPOINT', 'DFMCP_ORDER_RUN_ALLOW_CLOCK'}
    w.require(os.environ.get('DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14') == '1'
              and all(not k.startswith('DFMCP_') or k in allowed for k in os.environ), 'exact isolated developer environment required')
    w.require(os.environ.get('DFMCP_ORDER_RUN_ALLOW_CLOCK') in (None, '1'), 'clock opt-in must be absent or exactly 1')
    if control:
        w.require(os.environ.get('DFMCP_ORDER_RUN_ALLOW_CLOCK') == '1', 'operator clock authority required')
    endpoint = os.environ.get('DFMCP_ORDER_RUN_ENDPOINT', '127.0.0.1:5000'); w.address(endpoint)
    return endpoint, os.environ.get('DFMCP_ORDER_RUN_TOKEN', '').encode('utf-8')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='operation', required=True)
    for name in ('observe', 'prepare', 'commit', 'query', 'cancel', 'inspect'):
        cmd = commands.add_parser(name)
        if name != 'observe':
            cmd.add_argument('--journal', type=Path, required=True)
        if name not in ('observe', 'inspect'):
            cmd.add_argument('--key', required=True)
        if name != 'inspect':
            cmd.add_argument('--timeout-ms', type=int, default=10000)
        else:
            cmd.add_argument('--key'); cmd.add_argument('--after-key', default=''); cmd.add_argument('--head'); cmd.add_argument('--limit', type=int, default=4)
        if name in ('observe', 'prepare'):
            cmd.add_argument('--order-id', required=True, type=int)
        if name == 'prepare':
            cmd.add_argument('--world-folder', required=True); cmd.add_argument('--site-id', required=True, type=int)
            cmd.add_argument('--predicate', choices=('approved', 'active', 'remaining_at_most'), required=True)
            cmd.add_argument('--threshold', type=int, default=0); cmd.add_argument('--samples', type=int, default=1)
            cmd.add_argument('--interval', type=int, default=1); cmd.add_argument('--ticks', type=int, required=True)
            cmd.add_argument('--wall-ms', type=int, required=True)
        if name == 'commit':
            cmd.add_argument('--confirm-plan', required=True)
    args = parser.parse_args(argv)
    try:
        if args.operation == 'inspect':
            # Branch before environment, endpoint, credentials or socket access.
            with open_journal(args.journal) as journal:
                if args.key:
                    result = summary(journal.get(args.key))
                else:
                    w.integer(args.limit, 1, 8)
                    if args.after_key:
                        w.key(args.after_key); w.require(args.head == journal.head, 'continuation requires unchanged journal --head')
                    selected = [k for k in sorted(journal.entries) if k > args.after_key]
                    page = selected[:args.limit]
                    result = dict(journal_head=journal.head, total=len(journal.entries),
                                  unresolved=sum(unresolved(e) for e in journal.entries.values()),
                                  records=[summary(journal.entries[k]) for k in page],
                                  next_after=page[-1] if len(selected) > len(page) else None,
                                  historical_only=True, native_contacted=False)
        elif args.operation in ('observe', 'prepare'):
            endpoint, secret = environment(args.operation == 'prepare')
            w.integer(args.order_id, 0, 2**31 - 1)
            with w.Client(endpoint, secret, args.timeout_ms) as client:
                observed = client.call('ObserveRun', order_id=args.order_id)
                if args.operation == 'observe':
                    result = observed
                else:
                    w.require(observed['capture']['folder'] == args.world_folder and observed['capture']['site'] == args.site_id,
                              'observed fortress differs from explicit operator selection')
                    predicate = ('approved', 'active', 'remaining_at_most').index(args.predicate) + 1
                    encoded_plan = w.make_plan(bytes.fromhex(observed['capture_hex']), args.ticks, args.wall_ms,
                                               predicate, args.threshold, args.samples, args.interval)
                    result = prepare(client, args.journal, args.key, encoded_plan)
        else:
            endpoint, secret = environment(args.operation in ('commit', 'cancel'))
            with open_journal(args.journal, writable=True) as journal:
                w.require(endpoint == journal.header['binding']['endpoint'], 'operator endpoint differs from journal')
                entry = journal.get(args.key)
                # Pure local cancellation/terminal discovery works even if DFHack
                # is down; no socket is constructed for these branches.
                if args.operation != 'commit' and entry['state'] in ('terminal', 'cancelled_before_dispatch'):
                    result = summary(entry)
                elif args.operation == 'cancel' and entry['state'] in ('intent', 'prepared'):
                    entry = dict(entry, state='cancelled_before_dispatch'); journal.append(entry); result = summary(entry)
                else:
                    with w.Client(endpoint, secret, args.timeout_ms) as client:
                        result = operate(journal, client, args.operation, args.key, getattr(args, 'confirm_plan', None))
        payload = canonical(dict(ok=True, profile='order-run/1.14', runtime_admitted=False, result=result))
        w.require(len(payload) <= 128 * 1024, 'response too large; inspect recorded evidence with a smaller page')
        print(payload.decode()); return 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError) as error:
        print(json.dumps(dict(ok=False, profile='order-run/1.14', runtime_admitted=False, effect_status='unknown',
                              error_class=type(error).__name__, detail=str(error) if isinstance(error, w.Rejected)
                              else 'I/O or decoding failed; inspect the existing journal; never retry an unpause')))
        return 2


if __name__ == '__main__':
    sys.exit(main())
