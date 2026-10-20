"""Real private files/processes for identity-only furniture stop publication.

Fixtures are intentionally NOT placement evidence. The executable owner still
fully validates the original manifest and child journals before any native use.
"""
from __future__ import annotations

from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import request_furniture_batch_stop as command
import request_dig_blueprint_stop as custody

HEADER = b'{"schema":"dfmcp.furniture-batch-index/1"}\n'


def private(path, raw):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        if os.write(fd, raw) != len(raw):
            raise AssertionError('short fixture write')
    finally:
        os.close(fd)


def pair(path):
    info = path.stat()
    return [info.st_dev, info.st_ino]


@contextmanager
def fixture(schema='dfmcp.furniture-batch/1', **extra):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory) / 'batch'
        root.mkdir(mode=0o700)
        (root / 'effects').mkdir(mode=0o700)
        value = {'schema': schema, 'root_identity': pair(root),
                 'effects_identity': pair(root / 'effects'), **extra}
        raw = custody.envelope(value)
        private(root / command.MANIFEST, raw)
        private(root / command.INDEX, HEADER)
        yield root, hashlib.sha256(raw).hexdigest(), value


def stop_bytes(identity):
    # Independent expression of the existing furniture_batch.seal representation.
    value = {'schema': 'dfmcp.furniture-batch-stop/1', 'batch_id': identity}
    packed = json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()
    return json.dumps({'value': value, 'sha256': hashlib.sha256(packed).hexdigest()},
                      sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode() + b'\n'


def cli(root, identity, *extra):
    return subprocess.run([sys.executable, '-W', 'error', str(SCRIPTS / 'request_furniture_batch_stop.py'),
        '--directory', str(root), '--batch-id', identity, *extra], capture_output=True, timeout=10,
        env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})


class FurnitureStopTests(unittest.TestCase):
    def check_result(self, result, identity):
        self.assertTrue(result['ok'])
        self.assertEqual(result['batch_id'], identity)
        self.assertTrue(result['stop_request_verified'])
        self.assertTrue(result['future_step_admission_revoked'])
        self.assertTrue(result['in_flight_effect_may_complete'])
        self.assertEqual(result['native_calls'], 0)
        for field in ('native_effects_cancelled', 'native_quiescence_proven', 'game_paused_proven',
                      'inventory_verified', 'retry_commit_permitted', 'construction_completion_proven',
                      'production_admitted'):
            self.assertIs(result[field], False)
        self.assertIsNone(result['agent_turn']['anchor'])
        self.assertLessEqual(len(command.canonical(result)) + 1, command.MAX_OUTPUT)

    def test_existing_marker_bytes_and_manifest_hash_for_all_four_profiles(self):
        for schema in command.PROFILES:
            with self.subTest(schema=schema), fixture(schema) as (root, identity, _):
                original = {p.name: p.read_bytes() for p in root.iterdir() if p.is_file()}
                receipt = root / 'effects' / 'retained-original-key'
                private(receipt, b'historical-or-uncertain-original-bytes')
                result = command.request_stop(root, identity, command.Budget())
                self.check_result(result, identity)
                marker = root / command.STOP
                self.assertEqual(marker.read_bytes(), stop_bytes(identity))
                self.assertEqual(stat.S_IMODE(marker.stat().st_mode), 0o600)
                self.assertEqual(marker.stat().st_nlink, 1)
                self.assertEqual(result['stop_record_sha256'], hashlib.sha256(marker.read_bytes()).hexdigest())
                self.assertEqual({n: (root / n).read_bytes() for n in original}, original)
                self.assertEqual(receipt.read_bytes(), b'historical-or-uncertain-original-bytes')

    def test_cli_succeeds_while_separate_process_holds_every_executor_lock(self):
        code = '''
import fcntl,os,sys
from pathlib import Path
root=Path(sys.argv[1]); fds=[]
for path in (root,root/'effects',root/'batch.json',root/'steps.jsonl'):
 fd=os.open(path,os.O_RDONLY);fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB);fds.append(fd)
print('locked',flush=True)
sys.stdin.buffer.read(1)
for fd in fds:os.close(fd)
'''
        with fixture() as (root, identity, _):
            with subprocess.Popen([sys.executable, '-c', code, str(root)], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE) as owner:
                try:
                    self.assertTrue(select.select([owner.stdout], [], [], 5)[0])
                    self.assertEqual(owner.stdout.readline(), b'locked\n')
                    response = cli(root, identity)
                    self.assertIsNone(owner.poll(), 'executor unexpectedly exited before stop')
                    self.assertEqual(response.returncode, 0, response.stderr)
                    self.assertEqual(response.stderr, b'')
                    self.check_result(json.loads(response.stdout), identity)
                    self.assertEqual((root / command.STOP).read_bytes(), stop_bytes(identity))
                finally:
                    owner.communicate(b'x', timeout=5)

    def test_index_can_grow_while_stop_is_being_published(self):
        with fixture() as (root, identity, _):
            original = command.verify_batch
            calls = 0
            def append(*args):
                nonlocal calls
                calls += 1
                with (root / command.INDEX).open('ab') as out:
                    out.write(b'{"unverified_live_append":1}\n')
                return original(*args)
            with patch.object(command, 'verify_batch', side_effect=append):
                self.check_result(command.request_stop(root, identity, command.Budget()), identity)
            self.assertGreaterEqual(calls, 4)
            self.assertTrue((root / command.INDEX).read_bytes().startswith(HEADER))

    def test_same_completed_stop_is_resynced_without_write_or_inode_change(self):
        with fixture() as (root, identity, _):
            first = command.request_stop(root, identity, command.Budget())
            before = (root / command.STOP).stat()
            with patch.object(command.os, 'write', side_effect=AssertionError('second stop write')):
                second = command.request_stop(root, identity, command.Budget())
            self.assertEqual(first, second)
            after = (root / command.STOP).stat()
            self.assertEqual(custody.signature(before), custody.signature(after))

    def test_lost_file_or_directory_sync_acknowledgment_can_resync_same_record(self):
        for failed_call in (1, 2):
            with self.subTest(sync=failed_call), fixture() as (root, identity, _):
                real = os.fsync
                calls = 0
                def lost(fd):
                    nonlocal calls
                    calls += 1
                    real(fd)
                    if calls == failed_call:
                        raise OSError('lost synchronization acknowledgment')
                with patch.object(command.os, 'fsync', side_effect=lost), self.assertRaises(OSError):
                    command.request_stop(root, identity, command.Budget())
                self.assertEqual((root / command.STOP).read_bytes(), stop_bytes(identity))
                self.check_result(command.request_stop(root, identity, command.Budget()), identity)

    def test_partial_zero_or_failed_write_is_preserved_and_never_repaired(self):
        for mode in ('zero', 'short-then-error'):
            with self.subTest(mode=mode), fixture() as (root, identity, _):
                real, calls = os.write, 0
                def damaged(fd, raw):
                    nonlocal calls
                    calls += 1
                    if mode == 'zero':
                        return 0
                    if calls == 1:
                        return real(fd, raw[:7])
                    raise OSError('failed stop write')
                with patch.object(command.os, 'write', side_effect=damaged), self.assertRaises((ValueError, OSError)):
                    command.request_stop(root, identity, command.Budget())
                marker = (root / command.STOP).read_bytes()
                self.assertIn(len(marker), (0, 7))
                with self.assertRaises(ValueError):
                    command.request_stop(root, identity, command.Budget())
                self.assertEqual((root / command.STOP).read_bytes(), marker)

    def test_process_death_during_publication_keeps_torn_marker_and_original_files(self):
        code = '''
import os,signal,sys
from pathlib import Path
import request_furniture_batch_stop as c
write=os.write
def die(fd,raw):
 write(fd,raw[:11]);os.kill(os.getpid(),signal.SIGKILL)
os.write=die
c.request_stop(Path(sys.argv[1]),sys.argv[2],c.Budget())
'''
        with fixture() as (root, identity, _):
            before = (root / command.MANIFEST).read_bytes()
            process = subprocess.run([sys.executable, '-c', code, str(root), identity], capture_output=True,
                env={**os.environ, 'PYTHONPATH': str(SCRIPTS)}, timeout=10)
            self.assertEqual(process.returncode, -signal.SIGKILL)
            damaged = (root / command.STOP).read_bytes()
            self.assertEqual(len(damaged), 11)
            self.assertEqual(cli(root, identity).returncode, 2)
            self.assertEqual((root / command.STOP).read_bytes(), damaged)
            self.assertEqual((root / command.MANIFEST).read_bytes(), before)
            self.assertEqual((root / command.INDEX).read_bytes(), HEADER)

    def test_conflicting_and_unlocked_foreign_markers_are_not_overwritten(self):
        for raw in (stop_bytes('f' * 64), b'{}\n', b'x' * 2048):
            with self.subTest(raw=raw[:20]), fixture() as (root, identity, _):
                private(root / command.STOP, raw)
                with self.assertRaises(ValueError):
                    command.request_stop(root, identity, command.Budget())
                self.assertEqual((root / command.STOP).read_bytes(), raw)

    def test_competing_marker_owner_refuses_immediately(self):
        with fixture() as (root, identity, _):
            private(root / command.STOP, stop_bytes(identity))
            with (root / command.STOP).open('rb') as marker:
                fcntl.flock(marker, fcntl.LOCK_EX | fcntl.LOCK_NB)
                process = cli(root, identity)
            self.assertEqual(process.returncode, 2)
            self.assertEqual((root / command.STOP).read_bytes(), stop_bytes(identity))

    def test_wrong_identity_and_mutated_manifest_refuse_before_stop(self):
        with fixture() as (root, identity, value):
            with self.assertRaises(ValueError):
                command.request_stop(root, 'a' * 64, command.Budget())
            self.assertFalse((root / command.STOP).exists())
            (root / command.MANIFEST).write_bytes(custody.envelope({**value, 'changed': True}))
            with self.assertRaises(ValueError):
                command.request_stop(root, identity, command.Budget())
            self.assertFalse((root / command.STOP).exists())

    def test_copied_root_or_replaced_effects_refuse_before_stop(self):
        for which in ('root_identity', 'effects_identity'):
            with self.subTest(which=which), fixture() as (root, _, value):
                value[which][1] += 1
                raw = custody.envelope(value)
                (root / command.MANIFEST).write_bytes(raw)
                with self.assertRaises(ValueError):
                    command.request_stop(root, hashlib.sha256(raw).hexdigest(), command.Budget())
                self.assertFalse((root / command.STOP).exists())

    def test_replacement_at_publication_boundary_is_detected(self):
        for name in (command.MANIFEST, command.INDEX, command.EFFECTS):
            with self.subTest(name=name), fixture() as (root, identity, _):
                real, calls = command.verify_batch, 0
                def replaced(*args):
                    nonlocal calls
                    calls += 1
                    if calls == 2:
                        path = root / name
                        content = None if path.is_dir() else path.read_bytes()
                        path.rename(root.parent / 'old')
                        if content is None:
                            path.mkdir(mode=0o700)
                        else:
                            private(path, content)
                    return real(*args)
                with patch.object(command, 'verify_batch', side_effect=replaced), self.assertRaises(ValueError):
                    command.request_stop(root, identity, command.Budget())
                self.assertEqual((root / command.STOP).read_bytes(), b'')

    def test_foreign_inventory_missing_index_and_wrong_modes_refuse(self):
        for kind in ('extra', 'index-missing', 'index-empty', 'manifest-mode', 'index-mode', 'effects-mode', 'root-mode'):
            with self.subTest(kind=kind), fixture() as (root, identity, _):
                if kind == 'extra': private(root / 'unrelated', b'no')
                elif kind == 'index-missing': (root / command.INDEX).unlink()
                elif kind == 'index-empty': (root / command.INDEX).write_bytes(b'')
                else:
                    path = {'manifest-mode': root / command.MANIFEST, 'index-mode': root / command.INDEX,
                            'effects-mode': root / command.EFFECTS, 'root-mode': root}.get(kind)
                    path.chmod(0o755 if path.is_dir() else 0o644)
                with self.assertRaises((ValueError, OSError)):
                    command.request_stop(root, identity, command.Budget())
                self.assertFalse((root / command.STOP).exists())

    def test_special_linked_and_symlink_files_never_become_stop_targets(self):
        for name in (command.MANIFEST, command.INDEX, command.STOP):
            for kind in ('symlink', 'hardlink', 'fifo', 'directory'):
                with self.subTest(name=name, kind=kind), fixture() as (root, identity, _):
                    target = root / name
                    raw = target.read_bytes() if target.exists() else stop_bytes(identity)
                    if target.exists(): target.unlink()
                    other = root.parent / 'private-target'
                    private(other, raw)
                    if kind == 'symlink': target.symlink_to(other)
                    elif kind == 'hardlink': os.link(other, target)
                    elif kind == 'fifo': os.mkfifo(target, 0o600)
                    else: target.mkdir(mode=0o700)
                    with self.assertRaises((ValueError, OSError)):
                        command.request_stop(root, identity, command.Budget())
                    self.assertEqual(other.read_bytes(), raw)

    def test_symlink_parent_and_nonabsolute_root_refuse(self):
        with fixture() as (root, identity, _):
            alias = root.parent / 'alias'
            alias.symlink_to(root, target_is_directory=True)
            for selected in (alias, Path('.'), root.parent / '..' / root.parent.name / root.name):
                with self.subTest(path=selected), self.assertRaises((ValueError, OSError)):
                    command.request_stop(selected, identity, command.Budget())
            self.assertFalse((root / command.STOP).exists())

    def test_definition_bounds_are_per_profile_and_exact(self):
        base = {'schema': 'dfmcp.furniture-batch/1', 'root_identity': [1, 2], 'effects_identity': [3, 4]}
        for name in ('root_identity', 'effects_identity'):
            for value in ([True, 1], [1.0, 1], [-1, 1], [2**64, 1], [], '1,2'):
                with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                    command.definition(custody.envelope({**base, name: value}), command.Budget())
        for schema in (True, [], 'dfmcp.furniture-batch/5', None):
            with self.assertRaises(ValueError):
                command.definition(custody.envelope({**base, 'schema': schema}), command.Budget())
        for schema, (size, _) in command.PROFILES.items():
            value = {**base, 'schema': schema, 'opaque': 'x' * size}
            with self.assertRaises(ValueError):
                command.definition(custody.envelope(value), command.Budget())
            value['opaque'] = 'x' * (size - len(custody.envelope({**value, 'opaque': ''})))
            self.assertEqual(len(custody.envelope(value)), size)
            self.assertEqual(command.definition(custody.envelope(value), command.Budget()), value)

    def test_terrain_nesting_does_not_widen_legacy_profiles(self):
        for depth in (16, 17, 18, 19):
            nested = 'literal braces {[]} and quote " backslash \\'
            for _ in range(depth - 2): nested = [nested]
            for schema, (_, limit) in command.PROFILES.items():
                value = {'schema': schema, 'root_identity': [1, 2], 'effects_identity': [3, 4], 'opaque': nested}
                raw = custody.envelope(value)
                with self.subTest(depth=depth, schema=schema):
                    if depth <= limit: self.assertEqual(command.definition(raw, command.Budget()), value)
                    else:
                        with self.assertRaises(ValueError): command.definition(raw, command.Budget())

    def test_duplicates_nonfinite_corrupt_and_noncanonical_inputs_refuse(self):
        value = {'schema': 'dfmcp.furniture-batch/1', 'root_identity': [1, 2], 'effects_identity': [3, 4]}
        raw = custody.envelope(value)
        bad = (raw.replace(b'"root_identity":[1,2]', b'"root_identity":[1,2],"root_identity":[1,2]'),
               raw.replace(b'[1,2]', b'[NaN,2]'), raw + b'\n', raw[:-1], b'[' * 19 + b']' * 19,
               b'{"value":{},"sha256":"bad"}\n', b'', b'null', b'"unclosed', b'\xff')
        for data in bad:
            with self.subTest(raw=data[:40]), self.assertRaises(ValueError):
                command.definition(data, command.Budget())

    def test_expired_or_cancelled_budget_never_claims_success(self):
        with fixture() as (root, identity, _):
            for kind in ('work', 'deadline'):
                budget = command.Budget()
                if kind == 'work': budget.work_left = 0
                else: budget.deadline = 0
                with self.assertRaises(ValueError): command.request_stop(root, identity, budget)
                self.assertFalse((root / command.STOP).exists())
        # Interrupt every check in a small successful run; later checkpoints may
        # leave a complete or partial permanent record, never a success result.
        class Counter(command.Budget):
            def __init__(self, stop=None):
                super().__init__(); self.calls = 0; self.stop = stop
            def check(self):
                self.calls += 1
                if self.calls == self.stop: raise InterruptedError('cancel stop publication')
                return super().check()
        with fixture() as (root, identity, _):
            counter = Counter()
            command.request_stop(root, identity, counter)
        for stop in range(1, counter.calls + 1):
            with self.subTest(stop=stop), fixture() as (root, identity, _):
                with self.assertRaises(InterruptedError): command.request_stop(root, identity, Counter(stop))

    def test_no_credentials_or_socket_and_safe_cli_failure(self):
        with fixture() as (root, identity, _):
            import socket
            with patch.object(socket, 'socket', side_effect=AssertionError('native contact')):
                self.check_result(command.request_stop(root, identity, command.Budget()), identity)
            response = cli(root, 'not-an-identity')
            self.assertEqual(response.returncode, 2)
            self.assertEqual(response.stderr, b'')
            doc = json.loads(response.stdout)
            self.assertFalse(doc['ok'])
            self.assertFalse(doc['stop_request_verified'])
            self.assertNotIn(str(root).encode(), response.stdout)
            self.assertNotIn(b'not-an-identity', response.stdout)

    def test_short_stdout_writes_once_and_keeps_recorded_stop(self):
        with fixture() as (root, identity, _):
            outputs = []
            with patch.object(custody, 'publish', side_effect=lambda raw: outputs.append(raw) or False):
                code = command.main(['--directory', str(root), '--batch-id', identity])
            self.assertEqual(code, 2)
            self.assertEqual(len(outputs), 1)
            self.assertTrue(json.loads(outputs[0])['ok'])
            self.assertEqual((root / command.STOP).read_bytes(), stop_bytes(identity))


    def test_unsupported_platform_refuses_before_importing_posix_locking(self):
        import builtins
        original = builtins.__import__
        def no_locking(name, *args, **kwargs):
            if name == 'fcntl': raise AssertionError('premature platform-specific import')
            return original(name, *args, **kwargs)
        with fixture() as (root, identity, _):
            with patch.object(command.os, 'name', 'nt'), patch.object(builtins, '__import__', side_effect=no_locking):
                with self.assertRaisesRegex(ValueError, 'POSIX'):
                    command.request_stop(root, identity, command.Budget())
            self.assertFalse((root / command.STOP).exists())

    def test_marker_replacement_during_sync_is_not_acknowledged(self):
        with fixture() as (root, identity, _):
            real = os.fsync
            calls = 0
            def replaced(fd):
                nonlocal calls
                calls += 1
                real(fd)
                if calls == 1:
                    marker = root / command.STOP
                    marker.rename(root.parent / 'original-stop')
                    private(marker, stop_bytes(identity))
            with patch.object(command.os, 'fsync', side_effect=replaced), self.assertRaises(ValueError):
                command.request_stop(root, identity, command.Budget())
            self.assertEqual((root.parent / 'original-stop').read_bytes(), stop_bytes(identity))
            self.assertEqual((root / command.STOP).read_bytes(), stop_bytes(identity))

    def test_root_replacement_during_sync_does_not_write_to_new_root(self):
        with fixture() as (root, identity, _):
            real = os.fsync
            old = root.parent / 'old-batch'
            calls = 0
            def replaced(fd):
                nonlocal calls
                calls += 1
                real(fd)
                if calls == 1:
                    root.rename(old)
                    root.mkdir(mode=0o700)
            with patch.object(command.os, 'fsync', side_effect=replaced), self.assertRaises(ValueError):
                command.request_stop(root, identity, command.Budget())
            self.assertEqual(list(root.iterdir()), [])
            self.assertEqual((old / command.STOP).read_bytes(), stop_bytes(identity))

    def test_failed_publication_does_not_leak_open_descriptors(self):
        descriptor_dir = Path('/proc/self/fd')
        if not descriptor_dir.is_dir():
            # A process descriptor inventory is Linux-specific; the functional
            # failure tests above run independently of this extra assertion.
            return
        before = set(descriptor_dir.iterdir())
        for _ in range(20):
            with fixture() as (root, identity, _):
                with patch.object(command.os, 'fsync', side_effect=OSError('sync failed')):
                    with self.assertRaises(OSError):
                        command.request_stop(root, identity, command.Budget())
        self.assertEqual(set(descriptor_dir.iterdir()), before)

    def test_real_stdout_failure_keeps_permanent_stop_and_emits_no_second_packet(self):
        class Writer:
            def __init__(self): self.calls = 0; self.flushes = 0
            def write(self, raw): self.calls += 1; return len(raw) - 1
            def flush(self): self.flushes += 1
        class Output:
            def __init__(self): self.buffer = Writer()
        with fixture() as (root, identity, _):
            target = Output()
            with patch.object(custody.sys, 'stdout', target):
                code = command.main(['--directory', str(root), '--batch-id', identity])
            self.assertEqual(code, 2)
            self.assertEqual(target.buffer.calls, 1)
            self.assertEqual(target.buffer.flushes, 0)
            self.assertEqual((root / command.STOP).read_bytes(), stop_bytes(identity))

    def test_helpers_are_not_monkeypatched_or_profile_switched(self):
        original = (custody.MANIFEST, custody.EFFECTS, custody.STOP, custody.LEGACY_FORMAT,
                    custody.ROOM_FORMAT, custody.STOP_FORMAT)
        with fixture() as (root, identity, _): command.request_stop(root, identity, command.Budget())
        self.assertEqual((custody.MANIFEST, custody.EFFECTS, custody.STOP, custody.LEGACY_FORMAT,
                          custody.ROOM_FORMAT, custody.STOP_FORMAT), original)


if __name__ == '__main__':
    unittest.main()
