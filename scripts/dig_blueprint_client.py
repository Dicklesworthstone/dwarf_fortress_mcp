#!/usr/bin/env python3
"""Durable, explicitly stepped floor-blueprint designation using dig/1.16.

One advance means at most ONE existing native commit. Recovery never advances.
No unpause, subprocess, arbitrary RPC selector, implicit retry or rollback exists.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager, nullcontext
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import time

import dig_blueprint as b
import dig_designation_client as d
import dig_designation_store as s

MANIFEST = 'blueprint.json'
EFFECTS = 'effects'
STOP = 'stopped.json'
FORMAT = 'dfmcp.dig-blueprint-batch/1'
POLICY = 'disposable-fortress-no-checkpoint'
MAX_MANIFEST = 65536
MAX_OUTPUT = 131072


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


class Budget:
    def __init__(self, timeout_ms):
        d.integer(timeout_ms, 1, 60000)
        self.deadline = time.monotonic() + timeout_ms / 1000

    def check(self):
        d.require(time.monotonic() < self.deadline, 'blueprint command deadline exhausted')

    def milliseconds(self):
        self.check()
        return d.integer(int((self.deadline - time.monotonic()) * 1000), 1, 60000)


def file_bytes(value):
    return d.canonical({'value': value, 'sha256': sha(d.canonical(value))}) + b'\n'


@contextmanager
def private_file(directory, name, maximum, value=None):
    """Fixed-name immutable data under an already held private directory lock."""
    import fcntl
    directory.verify()
    flags = os.O_RDONLY if value is None else os.O_RDWR | os.O_CREAT | os.O_EXCL
    fd = os.open(name, flags | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                 0o600, dir_fd=directory.parent)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        before = os.fstat(fd)
        d.require(stat.S_ISREG(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o600
                  and before.st_nlink == 1 and before.st_uid == os.fstat(directory.parent).st_uid,
                  'invalid private blueprint file')
        if value is not None:
            raw = file_bytes(value)
            d.require(len(raw) <= maximum, 'blueprint file bound exceeded')
            view = memoryview(raw)
            while view:
                directory.budget()
                count = os.write(fd, view)
                d.require(count > 0, 'short blueprint write; preserve original bytes')
                view = view[count:]
            os.fsync(fd); os.fsync(directory.parent)
        else:
            d.require(1 <= before.st_size <= maximum, 'empty or oversized blueprint file; no repair')
            raw = bytearray()
            while len(raw) <= before.st_size:
                directory.budget()
                part = os.read(fd, before.st_size + 1 - len(raw))
                if not part:
                    break
                raw += part
            raw = bytes(raw)
            after = os.fstat(fd)
            d.require((before.st_mtime_ns, before.st_ctime_ns, before.st_size)
                      == (after.st_mtime_ns, after.st_ctime_ns, after.st_size), 'blueprint changed during read')
        loaded = b.bounded_json(raw, maximum)
        d.require(type(loaded) is dict and set(loaded) == {'value', 'sha256'}
                  and file_bytes(loaded['value']) == raw, 'invalid blueprint checksum or canonical encoding')
        pin = d.Capsule(directory.root / name, fd, directory.parent, raw, {})
        pin.verify(); directory.verify()
        yield loaded['value'], pin
        pin.verify(); directory.verify()
    finally:
        os.close(fd)


def source_of(observed):
    return {key: observed[key] for key in ('generation', 'folder', 'site', 'dimensions')}


def decode_manifest(value):
    d.require(type(value) is dict and set(value) == {'format', 'blueprint', 'layout_digest',
        'endpoint', 'manifest', 'bootstrap_hex', 'allow_hidden_neighbors', 'checkpoint_policy',
        'effects_identity'} and value['format'] == FORMAT and value['checkpoint_policy'] == POLICY,
        'invalid blueprint batch contract')
    layout = b.Layout.from_json(value['blueprint'])
    d.require(value['blueprint'] == layout.blueprint() and value['layout_digest'] == layout.digest,
              'blueprint layout changed')
    d.endpoint(value['endpoint']); d.manifest(value['manifest']); d.flag(value['allow_hidden_neighbors'])
    d.require(type(value['effects_identity']) is list and len(value['effects_identity']) == 2,
              'invalid effects directory identity')
    for n in value['effects_identity']:
        d.integer(n, 0, 2**64 - 1)
    observed = d.observation(d.exact_hex(value['bootstrap_hex'], 1, 16384), layout.regions()[0])
    d.require(observed['generation'] == value['manifest']['generation'] and observed['paused'],
              'invalid bootstrap source or clock')
    # Preflight every future halo against this source, not just the first room.
    for region in layout.regions():
        x, y, z, w, h = d.region(region)
        dx, dy, dz = observed['dimensions']
        d.require(x + w < dx and y + h < dy and z + 1 < dz, 'later blueprint halo outside fortress map')
    return layout, observed


class Batch:
    def __init__(self, directory, pin, value, budget):
        self.directory, self.pin, self.value, self.budget = directory, pin, value, budget
        self.layout, self.bootstrap = decode_manifest(value)
        self.id = d.digest(b'dfmcp-dig-blueprint-batch/1', d.canonical(value)).hex()
        self.stopped = False
        self.stop_pin = None

    @property
    def effects(self):
        return self.directory.root / EFFECTS

    def step_key(self, index):
        d.integer(index, 0, len(self.layout.rectangles) - 1)
        return f'bp-{self.id}-{index:03d}'

    def step_name(self, index):
        self.step_key(index)
        return f'step-{index:03d}.json'

    def verify(self):
        self.budget.check(); self.directory.verify(); self.pin.verify()
        expected = {MANIFEST, EFFECTS} | ({STOP} if self.stopped else set())
        d.require(set(self.directory.names()) == expected, 'unexpected or changed blueprint directory')
        child = os.stat(EFFECTS, dir_fd=self.directory.parent, follow_symlinks=False)
        d.require(stat.S_ISDIR(child.st_mode) and stat.S_IMODE(child.st_mode) == 0o700
                  and child.st_uid == os.fstat(self.directory.parent).st_uid
                  and [child.st_dev, child.st_ino] == self.value['effects_identity'],
                  'blueprint effects directory was replaced')
        if self.stop_pin is not None:
            self.stop_pin.verify()

    def check_observation(self, response, region, minimum_tick=None, minimum_sequence=None):
        self.verify()
        d.require(response['manifest'] == self.value['manifest'], 'blueprint native incarnation/software differs')
        observed = d.observation(response['raw'], region)
        d.require(source_of(observed) == source_of(self.bootstrap), 'blueprint fortress or dimensions differ')
        d.require(observed['tick'] >= (self.bootstrap['tick'] if minimum_tick is None else minimum_tick)
                  and observed['sequence'] >= (self.bootstrap['sequence'] if minimum_sequence is None else minimum_sequence),
                  'blueprint clock or intervention sequence regressed')
        return observed

    def inventory(self, store, current=None):
        self.verify()
        d.require(store.identity == tuple(self.value['effects_identity']), 'wrong blueprint effects owner')
        records = store.audit(current)
        d.require(len(records) <= len(self.layout.rectangles), 'unexpected blueprint native records')
        rows = []
        tick, sequence = self.bootstrap['tick'], self.bootstrap['sequence']
        regions = self.layout.regions()
        for index, record in enumerate(records):
            self.budget.check()
            d.require(record['name'] == self.step_name(index) and record['key'] == self.step_key(index)
                      and record['region'] == regions[index] and record['manifest'] == self.value['manifest']
                      and record['endpoint'] == self.value['endpoint'], 'out-of-order or substituted blueprint intent')
            d.require(not rows or rows[-1]['state'] == 'designated', 'unresolved or refused step was bypassed')
            context = nullcontext(current) if current is not None and current.path.name == record['name'] else d.capsule(self.effects / record['name'])
            with context as owner:
                d.require(sha(owner.raw) == record['intent_sha256']
                          and owner.intent['allow_hidden_neighbors'] == self.value['allow_hidden_neighbors'],
                          'blueprint intent or hidden-neighbor policy changed')
                observed = self.check_observation({'manifest': owner.intent['manifest'],
                    'raw': bytes.fromhex(owner.intent['observation_hex'])}, regions[index], tick, sequence)
                tick, sequence = observed['tick'], observed['sequence']
                if record['effect_status'] == 'designated':
                    sequence += 1
                owner.verify()
            rows.append({'step': index, 'key': record['key'], 'region': record['region'],
                'state': record['effect_status'], 'witness': owner.intent['witness'],
                'plan_digest': record['plan_digest'], 'intent_sha256': record['intent_sha256'],
                'terminal_receipt_sha256': record['terminal_receipt_sha256']})
        phase = ('unresolved' if rows[-1]['state'] == 'unknown' else 'refused') if rows and rows[-1]['state'] != 'designated' else (
            'designations_verified' if len(rows) == len(regions) else 'ready')
        state = {'batch_id': self.id, 'layout_digest': self.layout.digest,
            'blueprint_digest': self.layout.blueprint_digest, 'phase': phase, 'stopped': self.stopped,
            'total_steps': len(regions), 'target_tiles': len(self.layout.targets),
            'designated_steps': sum(r['state'] == 'designated' for r in rows),
            'next_step': len(rows) if phase == 'ready' and not self.stopped else None,
            'unresolved_step': len(rows) - 1 if phase == 'unresolved' else None,
            'minimum_tick': tick, 'minimum_sequence': sequence, 'records': rows,
            'excavation_completion_proven': False, 'current_terrain_proven': False,
            'global_controller_fence': False, 'checkpoint_verified': False, 'retry_commit_permitted': False}
        state['inventory_digest'] = d.digest(b'dfmcp-dig-blueprint-inventory/1', d.canonical(state)).hex()
        store.verify(); self.verify()
        return state


@contextmanager
def open_batch(root, budget):
    with s.open_store(d, root, budget=budget.check) as directory:
        with private_file(directory, MANIFEST, MAX_MANIFEST) as (value, pin):
            batch = Batch(directory, pin, value, budget)
            names = set(directory.names())
            d.require(names in ({MANIFEST, EFFECTS}, {MANIFEST, EFFECTS, STOP}), 'incomplete or unexpected batch directory')
            if STOP in names:
                with private_file(directory, STOP, 1024) as (stopped, stop_pin):
                    d.require(stopped == {'format': 'dfmcp.dig-blueprint-stop/1', 'batch_id': batch.id}, 'wrong batch stop marker')
                    batch.stopped, batch.stop_pin = True, stop_pin
                    batch.verify(); yield batch; batch.verify()
            else:
                batch.verify(); yield batch; batch.verify()


def initialize(root, layout, folder, site, allow_hidden, policy, budget, connect=d.Client):
    d.flag(allow_hidden); d.integer(site, 0, 2**31 - 1); d.utf8(folder.encode('utf-8'), 512)
    d.require(policy == POLICY, 'blueprint designation requires explicit disposable-fortress policy')
    # Reconstruct even an in-process Layout; forged dataclass fields are not admission.
    layout = b.Layout.decode(layout.blueprint_bytes)
    with s.open_store(d, root, budget=budget.check) as directory:
        d.require(not directory.names(), 'initialization requires an existing empty private directory')
        address, token = d.environment(False)
        with connect(address, token, budget.milliseconds()) as client:
            response = client.observe(layout.regions()[0])
            observed = d.observation(response['raw'], layout.regions()[0])
            d.require(observed['folder'] == folder and observed['site'] == site and observed['paused'],
                      'bootstrap does not identify the requested paused fortress')
            value = {'format': FORMAT, 'blueprint': layout.blueprint(), 'layout_digest': layout.digest,
                'endpoint': address, 'manifest': response['manifest'], 'bootstrap_hex': response['raw'].hex(),
                'allow_hidden_neighbors': allow_hidden, 'checkpoint_policy': policy, 'effects_identity': [0, 0]}
            decode_manifest(value)  # ALL extents fail before publishing any local store.
            directory.verify(); budget.check()
            os.mkdir(EFFECTS, 0o700, dir_fd=directory.parent)
            os.fsync(directory.parent)
            with s.open_store(d, root / EFFECTS, True, budget.check) as store:
                store.initialize()
                value['effects_identity'] = list(store.identity)
            with private_file(directory, MANIFEST, MAX_MANIFEST, value) as (_, pin):
                batch = Batch(directory, pin, value, budget)
                with s.open_store(d, batch.effects, budget=budget.check) as store:
                    result = batch.inventory(store)
                result.update(ok=True, native_mutation_dispatched=False)
                return result


def review_seal(batch, state, index, witness, plan):
    d.exact_hex(witness, 32); d.exact_hex(plan, 32)
    return d.digest(b'dfmcp-dig-blueprint-review/1', d.canonical({'batch_id': batch.id,
        'inventory': state['inventory_digest'], 'step': index, 'witness': witness, 'plan': plan})).hex()


class CheckedClient:
    def __init__(self, batch, client, credentials, region, state):
        self.batch, self.inner, self.credentials, self.region, self.state = batch, client, credentials, region, state
        self.address = client.address

    def check(self, control=False):
        self.batch.verify()
        d.require(d.environment(control, self.address) == self.credentials, 'blueprint operator selection changed')
        d.require(self.inner.manifest == self.batch.value['manifest'], 'blueprint native source differs')
        self.batch.budget.check()

    def remaining(self):
        self.check()
        return min(self.inner.remaining(), self.batch.budget.deadline - time.monotonic())

    def observe(self, selected):
        self.check()
        d.require(selected == self.region, 'blueprint rectangle changed')
        result = self.inner.observe(selected)
        self.batch.check_observation(result, selected, self.state['minimum_tick'], self.state['minimum_sequence'])
        return result

    def prepare(self, intent):
        self.check(True)
        d.require(not self.batch.stopped, 'blueprint was stopped')
        return self.inner.prepare(intent)

    def commit(self, intent, preparation):
        self.check(True)
        d.require(not self.batch.stopped, 'blueprint was stopped')
        return self.inner.commit(intent, preparation)


def inspect(root, budget):
    with open_batch(root, budget) as batch:
        with s.open_store(d, batch.effects, budget=budget.check) as store:
            result = batch.inventory(store)
            result.update(ok=True, native_calls=0, blueprint=batch.layout.blueprint())
            return result


def observe(root, budget, connect=d.Client):
    with open_batch(root, budget) as batch:
        with s.open_store(d, batch.effects, budget=budget.check) as store:
            state = batch.inventory(store)
            index = state['next_step']
            d.require(index is not None, 'blueprint is not ready; inspect or recover its existing work')
            region = batch.layout.regions()[index]
            credentials = d.environment(False, batch.value['endpoint'])
            with connect(*credentials, budget.milliseconds()) as raw:
                client = CheckedClient(batch, raw, credentials, region, state)
                native = client.observe(region)
                result = d.observed_result(native, batch.value['allow_hidden_neighbors'])
                plan, witness = result['plan_digest_for_confirmation'], result['observation']['witness']
                result.update(batch_id=batch.id, step=index, key=batch.step_key(index),
                    inventory_digest=state['inventory_digest'], layout_digest=batch.layout.digest,
                    review_seal=review_seal(batch, state, index, witness, plan))
                d.require(batch.inventory(store) == state, 'blueprint inventory changed during review')
                return result


def advance(root, batch_id, index, witness, plan, seal, budget, connect=d.Client):
    with open_batch(root, budget) as batch:
        d.require(batch_id == batch.id, 'wrong blueprint batch identity')
        d.integer(index, 0, len(batch.layout.rectangles) - 1)
        with s.open_store(d, batch.effects, budget=budget.check) as store:
            state = batch.inventory(store)
        d.require(state['next_step'] == index and seal == review_seal(batch, state, index, witness, plan),
                  'stale, unconfirmed or already attempted blueprint step')
        region = batch.layout.regions()[index]
        d.require(plan == d.plan_for(region, batch.value['allow_hidden_neighbors'], d.exact_hex(witness, 32)).hex(),
                  'blueprint confirmation differs from native plan')
        credentials = d.environment(True, batch.value['endpoint'])
        def guard(store, owner):
            current = batch.inventory(store, owner)
            if owner is None:
                d.require(current == state, 'blueprint changed before native preparation')
            else:
                d.require(len(current['records']) == index + 1 and current['records'][:index] == state['records']
                          and current['records'][index]['plan_digest'] == plan,
                          'blueprint changed across native effect boundary')
        with connect(*credentials, budget.milliseconds()) as raw:
            client = CheckedClient(batch, raw, credentials, region, state)
            result = s.start_designation(d, client, batch.effects / batch.step_name(index), batch.step_key(index),
                region, batch.value['allow_hidden_neighbors'], witness, plan, guard=guard)
            with s.open_store(d, batch.effects, budget=budget.check) as store:
                result['batch'] = batch.inventory(store)
            return result


def recover(root, batch_id, index, cancel, budget, connect=d.Client):
    d.flag(cancel)
    with open_batch(root, budget) as batch:
        d.require(batch_id == batch.id, 'wrong blueprint batch identity')
        with s.open_store(d, batch.effects, True, budget.check) as store:
            state = batch.inventory(store)
            d.integer(index, 0, len(state['records']) - 1)
            with d.capsule(batch.effects / batch.step_name(index)) as owner:
                retained = d.terminal_receipt(owner)
                if retained is not None:  # Branch before environment or native connection.
                    result = d.retained_result(owner, retained)
                else:
                    if not any(e['name'] == owner.path.name for e in store.entries):
                        store.register(owner)
                    credentials = d.environment(cancel, batch.value['endpoint'])
                    with connect(*credentials, budget.milliseconds()) as client:
                        batch.verify(); store.verify(); owner.verify()
                        d.require(d.environment(cancel, client.address) == credentials, 'operator changed during recovery')
                        budget.check()
                        native = client.cancel(owner.intent) if cancel else client.query(owner.intent)
                        result = d.finish_recovery(owner, native, False)
                result['batch'] = batch.inventory(store, owner)
                return result


def stop(root, batch_id, budget):
    with open_batch(root, budget) as batch:
        d.require(batch_id == batch.id, 'wrong blueprint batch identity')
        if not batch.stopped:
            value = {'format': 'dfmcp.dig-blueprint-stop/1', 'batch_id': batch.id}
            with private_file(batch.directory, STOP, 1024, value) as (_, pin):
                batch.stopped, batch.stop_pin = True, pin
                batch.verify()
                # open_batch's final verification occurs after this descriptor closes.
                batch.stop_pin = None
        return {'ok': True, 'batch_id': batch.id, 'future_steps_stopped': True,
                'native_calls': 0, 'native_effects_cancelled': False, 'game_paused_proven': False,
                'excavation_completion_proven': False, 'retry_commit_permitted': False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('init', 'inspect', 'observe', 'advance', 'query', 'cancel', 'stop'):
        p = commands.add_parser(name)
        p.add_argument('--directory', type=Path, required=True)
        p.add_argument('--timeout-ms', type=int, default=10000)
        if name == 'init':
            p.add_argument('--blueprint', type=Path, required=True)
            p.add_argument('--world-folder', required=True)
            p.add_argument('--site', type=int, required=True)
            p.add_argument('--allow-hidden-neighbors', action='store_true')
            p.add_argument('--checkpoint-policy', choices=[POLICY], required=True)
        if name in ('advance', 'query', 'cancel', 'stop'):
            p.add_argument('--batch-id', required=True)
        if name in ('advance', 'query', 'cancel'):
            p.add_argument('--step', type=int, required=True)
        if name == 'advance':
            p.add_argument('--expected-witness', required=True)
            p.add_argument('--confirm-plan', required=True)
            p.add_argument('--review-seal', required=True)
    args = parser.parse_args(argv)
    try:
        budget = Budget(args.timeout_ms)
        if args.command == 'init':
            with args.blueprint.open('rb') as source:
                layout = b.Layout.decode(source.read(b.MAX_INPUT + 1))
            result = initialize(args.directory, layout, args.world_folder, args.site,
                                args.allow_hidden_neighbors, args.checkpoint_policy, budget)
        elif args.command == 'inspect':
            result = inspect(args.directory, budget)
        elif args.command == 'observe':
            result = observe(args.directory, budget)
        elif args.command == 'stop':
            result = stop(args.directory, args.batch_id, budget)
        elif args.command == 'advance':
            result = advance(args.directory, args.batch_id, args.step, args.expected_witness,
                             args.confirm_plan, args.review_seal, budget)
        else:
            result = recover(args.directory, args.batch_id, args.step, args.command == 'cancel', budget)
        data = d.canonical(result)
        d.require(len(data) <= MAX_OUTPUT, 'complete blueprint response exceeds 128 KiB')
        budget.check()
        print(data.decode('ascii')); return 0
    except (ValueError, OSError, TypeError, KeyError, RecursionError):
        print(d.canonical({'ok': False, 'profile': 'dig-blueprint/1', 'effect_status': 'unknown',
            'error': 'Blueprint identity, custody, source, confirmation, deadline or native evidence refused. Preserve the batch and recover its original step.',
            'retry_commit_permitted': False, 'excavation_completion_proven': False}).decode('ascii'))
        return 2


if __name__ == '__main__':
    sys.exit(main())
