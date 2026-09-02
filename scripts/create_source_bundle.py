#!/usr/bin/env python3
"""Create one deterministic manifest-sealed source bundle from a clean commit."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, NoReturn

MODULE_PATH = Path(__file__).with_name("verify_source_bundle.py")
SPEC = importlib.util.spec_from_file_location("verify_source_bundle", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load source bundle verifier")
verifier = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verifier
SPEC.loader.exec_module(verifier)

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACT = ROOT / "architecture/source_bundle_v1.json"


class SourceBundleCreationError(ValueError):
    pass


def fail(message: str) -> NoReturn:
    raise SourceBundleCreationError(message)


def run_git(
    source_root: Path,
    arguments: list[str],
    *,
    binary: bool = False,
) -> bytes | str:
    try:
        completed = subprocess.run(
            ["git", "-C", os.fspath(source_root), *arguments],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=not binary,
        )
    except OSError as exc:
        fail(f"cannot execute Git: {exc}")
    if completed.returncode != 0:
        stderr = completed.stderr
        if isinstance(stderr, bytes):
            detail = stderr.decode("utf-8", errors="replace")
        else:
            detail = stderr
        fail(f"Git command failed: {detail.strip()[:1024]}")
    return completed.stdout


def require_clean_source(source_root: Path) -> None:
    status = run_git(
        source_root,
        ["status", "--porcelain=v1", "--untracked-files=all"],
    )
    if not isinstance(status, str):
        fail("Git status unexpectedly returned binary output")
    if status:
        fail("source bundle creation requires a clean worktree including untracked files")


def git_identity(source_root: Path) -> tuple[str, str]:
    commit_raw = run_git(source_root, ["rev-parse", "HEAD"])
    tree_raw = run_git(source_root, ["rev-parse", "HEAD^{tree}"])
    if not isinstance(commit_raw, str) or not isinstance(tree_raw, str):
        fail("Git identity unexpectedly returned binary output")
    return (
        verifier.require_commit(commit_raw.strip(), "git.head"),
        verifier.require_commit(tree_raw.strip(), "git.tree"),
    )


def path_is_within(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
    except ValueError:
        return False
    return True


def nearest_existing_parent(path: Path) -> tuple[Path, tuple[str, ...]]:
    cursor = path
    missing: list[str] = []
    while True:
        if cursor.is_symlink():
            fail("source bundle output path contains a symbolic-link parent")
        if cursor.exists():
            if not cursor.is_dir():
                fail("source bundle output parent is not a directory")
            return cursor.resolve(strict=True), tuple(reversed(missing))
        parent = cursor.parent
        if parent == cursor:
            fail("source bundle output path has no existing directory ancestor")
        missing.append(cursor.name)
        cursor = parent


def validate_output_location(source_root: Path, output_dir: Path) -> Path:
    if not output_dir.is_absolute():
        fail("output directory path must be absolute")
    raw = os.fspath(output_dir)
    if not raw or len(os.fsencode(raw)) > 4096:
        fail("output directory path is empty or exceeds its byte bound")
    if any(ord(character) < 0x20 for character in raw):
        fail("output directory path contains a control character")

    source = source_root.resolve(strict=True)
    lexical = Path(os.path.abspath(raw))
    if lexical.exists() or lexical.is_symlink():
        fail("source bundle output directory already exists")

    existing, missing = nearest_existing_parent(lexical.parent)
    expected_parent = existing.joinpath(*missing)
    candidate = expected_parent / lexical.name
    if candidate == source or path_is_within(candidate, source / ".git"):
        fail("source bundle output cannot be the source root or lie inside .git")
    if path_is_within(candidate, source):
        probe = subprocess.run(
            [
                "git",
                "-C",
                os.fspath(source),
                "check-ignore",
                "-q",
                "--no-index",
                os.fspath(lexical),
            ],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if probe.returncode != 0:
            fail("an output directory inside the source root must be ignored by Git")

    lexical.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    if lexical.parent.is_symlink():
        fail("source bundle output path contains a symbolic-link parent")
    actual_parent = lexical.parent.resolve(strict=True)
    if actual_parent != expected_parent:
        fail("source bundle output parent changed while being prepared")
    candidate = actual_parent / lexical.name
    if candidate.exists() or candidate.is_symlink():
        fail("source bundle output directory already exists")
    if candidate == source or path_is_within(candidate, source / ".git"):
        fail("source bundle output cannot be the source root or lie inside .git")
    return candidate


def stream_git_archive(
    source_root: Path,
    commit: str,
    prefix: str,
    destination: Path,
    maximum_bytes: int,
) -> int:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    descriptor = os.open(destination, flags, 0o600)
    process: subprocess.Popen[bytes] | None = None
    total = 0
    try:
        process = subprocess.Popen(
            [
                "git",
                "-C",
                os.fspath(source_root),
                "-c",
                "tar.umask=0022",
                "archive",
                "--format=tar",
                f"--prefix={prefix}",
                commit,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        if process.stdout is None or process.stderr is None:
            fail("Git archive did not provide bounded output pipes")
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            descriptor = -1
            while True:
                chunk = process.stdout.read(1024 * 1024)
                if not chunk:
                    break
                total += len(chunk)
                if total > maximum_bytes:
                    process.kill()
                    fail("source archive exceeds the contract byte bound")
                handle.write(chunk)
            handle.flush()
            os.fsync(handle.fileno())
        stderr = process.stderr.read()
        return_code = process.wait()
        if return_code != 0:
            detail = stderr.decode("utf-8", errors="replace").strip()
            fail(f"Git archive failed: {detail[:1024]}")
        if total == 0:
            fail("Git archive produced an empty source bundle")
        return total
    except BaseException:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        try:
            destination.unlink()
        except OSError:
            pass
        raise
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def write_json(path: Path, value: dict[str, Any]) -> None:
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    descriptor = os.open(path, flags, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
    except BaseException:
        try:
            path.unlink()
        except OSError:
            pass
        raise


def fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    descriptor = os.open(path, flags)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def build_manifest(
    source_root: Path,
    contract: dict[str, Any],
    commit: str,
    tree: str,
    archive_path: Path,
) -> dict[str, Any]:
    actual_tree, entries = verifier.git_entries(source_root, commit)
    if actual_tree != tree:
        fail("Git tree changed while the source bundle was being prepared")
    archive = verifier.read_stable_regular_file(
        archive_path,
        verifier.require_positive_int(
            contract["archive"]["maximum_bytes"],
            "contract.archive.maximum_bytes",
        ),
        "candidate source archive",
    )
    unsigned: dict[str, Any] = {
        "schema": verifier.MANIFEST_SCHEMA,
        "status": "created",
        "repository": "dwarf_fortress_mcp",
        "commit": commit,
        "tree": tree,
        "archive": {
            "name": archive_path.name,
            "format": "tar",
            "prefix": f"dwarf_fortress_mcp-{commit}/",
            "bytes": archive.size,
            "sha256": archive.sha256,
        },
        "entries": entries,
        "entries_digest": verifier.sha256_bytes(verifier.canonical_json(entries)),
        "claims_not_established": list(contract["claims_not_established"]),
    }
    return {
        **unsigned,
        "manifest_digest": verifier.sha256_bytes(verifier.canonical_json(unsigned)),
    }


def create_bundle(
    source_root: Path,
    output_dir: Path,
    contract_path: Path = DEFAULT_CONTRACT,
) -> dict[str, Any]:
    source = source_root.resolve(strict=True)
    if not source.is_dir():
        fail("source root is not a directory")
    require_clean_source(source)
    commit, tree = git_identity(source)
    destination = validate_output_location(source, output_dir)
    contract = verifier.load_contract(contract_path)
    maximum_archive_bytes = verifier.require_positive_int(
        contract["archive"]["maximum_bytes"],
        "contract.archive.maximum_bytes",
    )

    staging = Path(
        tempfile.mkdtemp(
            prefix=f".{destination.name}.",
            dir=destination.parent,
        )
    )
    archive_name = f"dwarf_fortress_mcp-{commit}.tar"
    archive_path = staging / archive_name
    manifest_path = staging / "source-bundle-manifest.json"
    verification_path = staging / "source-bundle-verification.json"
    try:
        stream_git_archive(
            source,
            commit,
            f"dwarf_fortress_mcp-{commit}/",
            archive_path,
            maximum_archive_bytes,
        )
        manifest = build_manifest(source, contract, commit, tree, archive_path)
        write_json(manifest_path, manifest)
        verification = verifier.verify(
            manifest_path,
            archive_path,
            contract_path,
            source,
            True,
        )
        write_json(verification_path, verification)
        require_clean_source(source)
        final_commit, final_tree = git_identity(source)
        if final_commit != commit or final_tree != tree:
            fail("source commit or tree changed during source bundle creation")
        for path in [archive_path, manifest_path, verification_path]:
            os.chmod(path, 0o644)
            descriptor = os.open(path, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        os.chmod(staging, 0o755)
        fsync_directory(staging)
        os.replace(staging, destination)
        fsync_directory(destination.parent)
        return {
            "schema": "dfmcp.source-bundle-creation/1",
            "status": "created_and_verified",
            "commit": commit,
            "tree": tree,
            "output_directory": os.fspath(destination),
            "archive": os.fspath(destination / archive_name),
            "manifest": os.fspath(destination / manifest_path.name),
            "verification": os.fspath(destination / verification_path.name),
            "archive_sha256": manifest["archive"]["sha256"],
            "manifest_digest": manifest["manifest_digest"],
            "entries_digest": manifest["entries_digest"],
            "entry_count": len(manifest["entries"]),
            "authority": {
                "executes_project_code": False,
                "modifies_source": False,
                "network_access": False,
                "grants_capabilities": [],
                "mutation_capabilities": [],
            },
        }
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "output_dir",
        nargs="?",
        type=Path,
        help="absolute output directory; defaults under target/source-bundle/<commit>",
    )
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        source = args.source_root.resolve(strict=True)
        commit, _ = git_identity(source)
        output = args.output_dir
        if output is None:
            output = source / "target" / "source-bundle" / commit
        elif not output.is_absolute():
            fail("output directory must be absolute")
        result = create_bundle(source, output, args.contract)
    except (
        OSError,
        subprocess.SubprocessError,
        verifier.StableReadError,
        verifier.SourceBundleError,
        SourceBundleCreationError,
    ) as exc:
        print(f"source bundle creation: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
