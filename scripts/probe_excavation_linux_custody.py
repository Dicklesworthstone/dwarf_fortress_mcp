#!/usr/bin/env python3
"""Exercise required Linux/POSIX primitives from Python, NOT the Rust backend.

Retains all probe files. No game, Rust, crash/power-loss durability or production
qualification is established by these host-kernel experiments.
"""
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import time


def require(ok, message):
    if not ok:
        raise AssertionError(message)


@contextmanager
def opened(path, flags, mode=0o600):
    fd = os.open(path, flags, mode)
    try:
        yield fd
    finally:
        os.close(fd)


def main():
    require(sys.platform == 'linux', 'this primitive experiment requires Linux')
    root = Path(tempfile.mkdtemp(prefix='dfmcp-custody-primitives-')).resolve()
    root.chmod(0o700)
    results = []
    directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    file_flags = os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    require(os.O_NOFOLLOW == 0x20000 and os.O_NONBLOCK == 0x800 and os.O_DIRECTORY == 0x10000,
            'Linux flag assumptions differ on this architecture')
    results.append('linux_flag_values')
    with opened(root, directory_flags) as directory:
        pinned = Path(f'/proc/self/fd/{directory}')
        require(os.stat(pinned).st_ino == os.stat(root).st_ino, 'proc-fd is not original directory')
        results.append('proc_fd_directory_identity')
        fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
        child = subprocess.run([sys.executable, '-c',
            'import os,fcntl,sys\nf=os.open(sys.argv[1],os.O_RDONLY|os.O_DIRECTORY)\n'
            'try:\n fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)\nexcept BlockingIOError:\n sys.exit(0)\n'
            'sys.exit(1)\n', str(root)], capture_output=True, text=True, timeout=3, check=False)
        require(child.returncode == 0, 'directory lock did not exclude another process')
        results.append('cross_process_directory_lock')
        name = pinned / 'journal'
        with opened(name, file_flags | os.O_CREAT | os.O_EXCL) as file:
            fcntl.flock(file, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with opened(name, file_flags) as second:
                try:
                    fcntl.flock(second, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    pass
                else:
                    raise AssertionError('separate file open reacquired exclusive lock')
            results.append('distinct_open_file_lock')
            os.write(file, b'complete-intent\n')
            os.fsync(file)
            os.fsync(directory)
            os.lseek(file, 0, os.SEEK_SET)
            require(os.read(file, 100) == b'complete-intent\n', 'synced write did not read back')
            results.append('file_and_directory_sync_then_readback')
            info = os.fstat(file)
            require(stat.S_IMODE(info.st_mode) == 0o600 and info.st_nlink == 1, 'private file metadata')
            # Owner-created aliases are observable via nofollow and link counts.
            (root / 'alias').symlink_to(root / 'journal')
            try:
                with opened(pinned / 'alias', file_flags):
                    raise AssertionError('nofollow admitted symlink')
            except OSError:
                pass
            os.link(root / 'journal', root / 'hard')
            require(os.fstat(file).st_nlink == 2, 'hardlink was not visible on pinned file')
            results.append('symlink_refusal_and_hardlink_detection')
        os.mkfifo(root / 'fifo', 0o600)
        started = time.monotonic()
        with opened(root / 'fifo', file_flags) as fifo:
            require(stat.S_ISFIFO(os.fstat(fifo).st_mode), 'FIFO classification failed')
        require(time.monotonic() - started < 1, 'nonblocking FIFO open blocked')
        results.append('nonblocking_special_file_classification')
        moved = root.with_name(root.name + '-retained')
        root.rename(moved)
        root.mkdir(mode=0o700)
        require(os.stat(pinned).st_ino == os.stat(moved).st_ino
                and os.stat(pinned).st_ino != os.stat(root).st_ino, 'directory replacement redirected pinned fd')
        with opened(pinned / 'pinned-only', file_flags | os.O_CREAT | os.O_EXCL) as file:
            os.write(file, b'original-directory-only')
        require((moved / 'pinned-only').exists() and not (root / 'pinned-only').exists(),
                'descriptor-relative write reached replacement directory')
        results.append('replacement_directory_is_not_pinned_directory')
    print(json.dumps({'result': 'passed', 'scope': 'Python execution of Linux primitives, not Rust code',
                      'checks': results, 'checks_count': len(results),
                      'retained_directories': [str(root), str(moved)],
                      'rust_compiled': False, 'rust_executed': False,
                      'physical_power_loss_tested': False, 'live_fortress': False}, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
