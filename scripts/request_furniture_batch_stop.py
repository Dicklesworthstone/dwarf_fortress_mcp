#!/usr/bin/env python3
"""Request a permanent furniture-batch stop without the executor's locks.

Only the existing stop.json record is published. Original manifest identity is
verified, but no room, terrain, allocation or effect history is admitted here.
An admitted/in-flight placement may finish; original-key recovery stays separate.
Beads: df-dfhack-bridge-plane-c-pic.4/.5; df-cancellation-progress-certificates-vvw.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import stat

# Reuse the already existing no-follow, descriptor-pinned stop-file primitives.
# Do not import the placement client, obtain credentials, or touch global state.
import request_dig_blueprint_stop as custody

MANIFEST = 'batch.json'
INDEX = 'steps.jsonl'
EFFECTS = 'effects'
STOP = 'stop.json'
STOP_SCHEMA = 'dfmcp.furniture-batch-stop/1'
SCHEMA = 'dfmcp.furniture-batch-stop-request/1'
MAX_OUTPUT = 8192
MAX_INDEX = 32768
PROFILES = {
    'dfmcp.furniture-batch/1': (32768, 16),
    'dfmcp.furniture-batch/2': (65536, 16),
    'dfmcp.furniture-batch/3': (128 * 1024, 16),
    'dfmcp.furniture-batch/4': (384 * 1024, 18),
}
MAX_MANIFEST = max(size for size, _ in PROFILES.values())
Budget = custody.Budget
require = custody.require
canonical = custody.canonical


def definition(raw: bytes, budget: Budget) -> dict:
    """Validate original canonical identity, not semantic work or proof eligibility."""
    require(type(raw) is bytes and 1 <= len(raw) <= MAX_MANIFEST,
            'furniture definition exceeds bound')
    depth = peak = 0
    quoted = escaped = False
    for offset, byte in enumerate(raw):
        if offset % 256 == 0:
            budget.check()
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
            peak = max(peak, depth)
            require(peak <= 18, 'furniture definition nesting exceeds bound')
        elif byte in (93, 125):
            depth -= 1
            require(depth >= 0, 'unbalanced furniture definition')
    require(depth == 0 and not quoted, 'incomplete furniture definition')

    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, 'duplicate furniture definition field')
            result[key] = value
        return result

    def nonfinite(_value):
        raise ValueError('nonfinite furniture definition number')

    envelope = json.loads(raw.decode('utf-8'), object_pairs_hook=unique, parse_constant=nonfinite)
    require(type(envelope) is dict and set(envelope) == {'value', 'sha256'}
            and type(envelope['value']) is dict and custody.envelope(envelope['value']) == raw,
            'noncanonical or corrupt furniture definition')
    value = envelope['value']
    schema = value.get('schema')
    require(type(schema) is str and schema in PROFILES, 'unsupported furniture batch profile')
    size_limit, depth_limit = PROFILES[schema]
    require(len(raw) <= size_limit and peak <= depth_limit, 'definition exceeds original profile')
    for name in ('root_identity', 'effects_identity'):
        selected = value.get(name)
        require(type(selected) is list and len(selected) == 2
                and all(type(n) is int and 0 <= n < 2**64 for n in selected),
                'invalid original furniture directory identity')
    budget.check()
    return value


@dataclass(frozen=True)
class IndexPin:
    """Pin identity only: the executor is allowed to append its original index.

    Never read, hash, flock or freeze the live index's contents. Such a snapshot
    could race a synchronized append and is unnecessary for revoking new work.
    This pin grants no journal integrity or complete-effect-inventory claim.
    """
    owner: custody.Directory
    fd: int
    identity: tuple[int, int]

    def verify(self) -> None:
        self.owner.verify()
        opened = os.fstat(self.fd)
        named = os.stat(INDEX, dir_fd=self.owner.fd, follow_symlinks=False)
        for info in (opened, named):
            require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600
                    and info.st_uid == self.owner.uid and info.st_nlink == 1
                    and (info.st_dev, info.st_ino) == self.identity
                    and 1 <= info.st_size <= MAX_INDEX, 'original furniture index custody changed')
        self.owner.verify()


@contextmanager
def index_pin(owner: custody.Directory):
    owner.verify()
    fd = os.open(INDEX, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=owner.fd)
    try:
        info = os.fstat(fd)
        pin = IndexPin(owner, fd, (info.st_dev, info.st_ino))
        pin.verify()
        yield pin
        pin.verify()
    finally:
        os.close(fd)


def verify_batch(owner: custody.Directory, manifest: custody.Pin,
                 index: IndexPin, value: dict) -> None:
    manifest.verify()
    index.verify()
    require(list(owner.identity) == value['root_identity'], 'furniture batch was moved or copied')
    names = set()
    with os.scandir(owner.fd) as entries:
        for entry in entries:
            owner.budget.check()
            require(len(names) < 4, 'unexpected furniture batch inventory')
            names.add(entry.name)
    require({MANIFEST, INDEX, EFFECTS} <= names <= {MANIFEST, INDEX, EFFECTS, STOP},
            'incomplete or foreign furniture batch inventory')
    info = os.stat(EFFECTS, dir_fd=owner.fd, follow_symlinks=False)
    require(stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o700
            and info.st_uid == owner.uid and [info.st_dev, info.st_ino] == value['effects_identity'],
            'original furniture effects directory was replaced')
    owner.verify()


def packet(batch_id: str | None = None, stop_sha256: str | None = None) -> dict:
    success = stop_sha256 is not None
    value = {'schema': SCHEMA, 'ok': success, 'batch_id': batch_id,
        'stop_request_verified': success, 'stop_record_sha256': stop_sha256,
        'future_step_admission_revoked': success, 'cooperative_only': True,
        'in_flight_effect_may_complete': True, 'native_calls': 0,
        'native_effects_cancelled': False, 'native_quiescence_proven': False,
        'game_paused_proven': False, 'inventory_verified': False,
        'retry_commit_permitted': False, 'construction_completion_proven': False,
        'production_admitted': False}
    if not success:
        value['error'] = {'code': 'FURNITURE_STOP_REQUEST_REFUSED', 'detail':
            'Preserve the original batch and any stop marker. Failed acknowledgment '
            'does not prove no stop was recorded. Recover uncertain original keys separately.'}
    value['agent_turn'] = {
        'schema': 'dfmcp.agent_turn/1', 'operation': 'furniture_batch.request_stop',
        'phase': 'inspect', 'session_id': None, 'request_id': None, 'anchor': None,
        'continuity': {'status': 'indeterminate', 'basis': None, 'gap': None},
        'briefing': {'stop_request_verified': success, 'runtime_admitted': False,
                     'mutation_admissible': False, 'native_quiescence_proven': False},
        'changes': [], 'attention': [], 'active_work': {'batch_id': batch_id,
            'effect_inventory_verified': False, 'in_flight_effect_may_complete': True},
        'affordances': [], 'recommendations': [],
        'uncertainty': ['A local stop record revokes future admission; it is not native cancellation, '
                        'game pause, rollback, construction completion or proof that work drained.',
                        'The original placement keys and receipts remain the only recovery path.'],
        'coverage': {'scope': 'original_furniture_batch_stop_record_only', 'native_observations': 0,
                     'semantic_manifest_validated': False},
        'budget': {'output_bytes_limit': MAX_OUTPUT, 'token_count_measured': False},
        'references': [] if batch_id is None else [batch_id]}
    return value


def request_stop(root: Path, batch_id: str, budget: Budget) -> dict:
    """Publish/re-sync exactly the existing permanent record, without owner locks."""
    require(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'furniture stop requires POSIX custody')
    import fcntl
    custody.identity(batch_id)
    expected = custody.envelope({'schema': STOP_SCHEMA, 'batch_id': batch_id})
    result = packet(batch_id, hashlib.sha256(expected).hexdigest())
    require(len(canonical(result)) + 1 <= MAX_OUTPUT, 'complete stop result exceeds output bound')
    with custody.directory(root, budget) as owner:
        with custody.pinned(owner, MANIFEST, MAX_MANIFEST) as manifest, index_pin(owner) as index:
            value = definition(manifest.raw, budget)
            # Batch.id is the hash of the EXACT sealed manifest bytes (including
            # its newline), unlike the dig batch's domain-separated value hash.
            require(hashlib.sha256(manifest.raw).hexdigest() == batch_id, 'different original furniture batch')
            verify_batch(owner, manifest, index, value)
            flags = os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
            created = False
            try:
                fd = os.open(STOP, os.O_RDWR | os.O_CREAT | os.O_EXCL | flags, 0o600, dir_fd=owner.fd)
                created = True
            except FileExistsError:
                fd = os.open(STOP, os.O_RDONLY | flags, dir_fd=owner.fd)
            try:
                # Never acquire directory, index, effect or terrain-journal locks.
                # Competing signal writers fail immediately instead of waiting.
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                info = os.fstat(fd)
                require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600
                        and info.st_uid == owner.uid and info.st_nlink == 1, 'invalid private furniture stop file')
                verify_batch(owner, manifest, index, value)
                if created:
                    pending = memoryview(expected)
                    while pending:
                        budget.check()
                        count = os.write(fd, pending)
                        require(count > 0, 'short furniture stop write; preserve partial record')
                        pending = pending[count:]
                else:
                    require(info.st_size == len(expected), 'incomplete or foreign stop record; no repair')
                    raw = custody.read_bytes(fd, info.st_size, budget)
                    require(custody.signature(os.fstat(fd)) == custody.signature(info) and raw == expected,
                            'conflicting furniture stop record; no overwrite')
                stop = custody.Pin(owner, STOP, fd, expected)
                verify_batch(owner, manifest, index, value)
                budget.check()
                os.fsync(fd)
                budget.check()
                os.fsync(owner.fd)
                stop.verify()
                verify_batch(owner, manifest, index, value)
            finally:
                os.close(fd)
    budget.check()
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        parser = custody.Parser(description=__doc__, allow_abbrev=False)
        parser.add_argument('--directory', type=Path, required=True)
        parser.add_argument('--batch-id', required=True)
        parser.add_argument('--timeout-ms', type=int, default=10000)
        args = parser.parse_args(argv)
        budget = Budget(args.timeout_ms)
        result = request_stop(args.directory, args.batch_id, budget)
        raw = canonical(result) + b'\n'
        budget.check()
        code = 0
    except (OSError, ValueError, TypeError, KeyError, RecursionError, KeyboardInterrupt):
        raw, code = canonical(packet()) + b'\n', 2
    # Partial stdout never repeats publication or emits a second response.
    return code if custody.publish(raw) else 2


if __name__ == '__main__':
    raise SystemExit(main())
