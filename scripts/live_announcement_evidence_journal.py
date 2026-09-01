#!/usr/bin/env python3
"""Capture an exact protocol-1.1 A1-A6 campaign transactionally."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import re
import stat
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterator

ROOT = Path(__file__).resolve().parents[1]
VERIFIER_PATH = ROOT / "scripts/verify_live_announcement_acceptance.py"
DEFAULT_ACCEPTANCE = ROOT / "architecture/live_announcement_acceptance_v1_1.json"
DEFAULT_JOURNAL_CONTRACT = ROOT / "architecture/live_announcement_evidence_journal_v1.json"
JOURNAL_SCHEMA = "dfmcp.live-announcement-evidence-journal/1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_JOURNAL_BYTES = 64 * 1024 * 1024
MAX_ASSERTIONS_BYTES = 1024 * 1024
MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
MAX_PATH_BYTES = 4096

SPEC = importlib.util.spec_from_file_location(
    "verify_live_announcement_acceptance", VERIFIER_PATH
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load live announcement acceptance verifier")
verifier = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = verifier
SPEC.loader.exec_module(verifier)


class JournalError(ValueError):
    pass


def fail(message: str) -> None:
    raise JournalError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")


def require_hash(value: str, path: str) -> str:
    if HEX64.fullmatch(value) is None or value == "0" * 64:
        fail(f"{path} must be a nonzero lowercase SHA-256 digest")
    return value


def ensure_path_bound(path: Path, label: str) -> None:
    raw = os.fspath(path)
    if not raw or len(os.fsencode(raw)) > MAX_PATH_BYTES:
        fail(f"{label} path is empty or exceeds its byte bound")
    if "\x00" in raw:
        fail(f"{label} path contains a NUL byte")


def stable_regular_bytes(path: Path, maximum: int, label: str) -> tuple[bytes, os.stat_result]:
    ensure_path_bound(path, label)
    if not hasattr(os, "O_NOFOLLOW"):
        fail("this platform cannot enforce no-follow evidence custody")
    flags = os.O_RDONLY | os.O_NOFOLLOW
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        fail(f"cannot open {label}: {exc}")
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            fail(f"{label} must be a regular file")
        if before.st_size <= 0 or before.st_size > maximum:
            fail(f"{label} must contain 1..={maximum} bytes, got {before.st_size}")
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(descriptor, min(1024 * 1024, maximum + 1 - total))
            if not chunk:
                break
            total += len(chunk)
            if total > maximum:
                fail(f"{label} grew beyond its byte bound while being read")
            chunks.append(chunk)
        after = os.fstat(descriptor)
        if (
            before.st_dev != after.st_dev
            or before.st_ino != after.st_ino
            or before.st_size != after.st_size
            or before.st_mtime_ns != after.st_mtime_ns
            or total != before.st_size
        ):
            fail(f"{label} changed while being read")
        return b"".join(chunks), after
    finally:
        os.close(descriptor)


def stable_file_sha256(path: Path, maximum: int, label: str) -> str:
    raw, _ = stable_regular_bytes(path, maximum, label)
    return sha256_bytes(raw)


def owner_private_mode(metadata: os.stat_result, label: str) -> None:
    mode = stat.S_IMODE(metadata.st_mode)
    if mode != 0o600:
        fail(f"{label} must have exact mode 0600, got {mode:04o}")
    if hasattr(os, "geteuid") and metadata.st_uid not in {0, os.geteuid()}:
        fail(f"{label} is not owned by root or the effective user")


def read_private_object(path: Path, label: str) -> tuple[dict[str, Any], str]:
    raw, metadata = stable_regular_bytes(path, MAX_JOURNAL_BYTES, label)
    owner_private_mode(metadata, label)
    return verifier.parse_object(raw, label), sha256_bytes(raw)


def validate_contract_file(path: Path, maximum: int, label: str) -> tuple[dict[str, Any], str]:
    raw, _ = stable_regular_bytes(path, maximum, label)
    return verifier.parse_object(raw, label), sha256_bytes(raw)


def load_journal_contract(path: Path) -> dict[str, Any]:
    value, _ = validate_contract_file(path, 1024 * 1024, "journal contract")
    verifier.require_exact_keys(
        value,
        {
            "schema_version",
            "journal_schema",
            "status",
            "acceptance_contract",
            "commands",
            "custody",
            "identity",
            "append_semantics",
            "export_semantics",
            "bounds",
            "authority",
            "claims_not_established",
        },
        "journal_contract",
    )
    if value.get("schema_version") != "dfmcp.live-announcement-evidence-journal-contract/1":
        fail("journal contract schema is unsupported")
    if value.get("journal_schema") != JOURNAL_SCHEMA:
        fail("journal schema drifted")
    if value.get("status") != "normative_capture_custody_contract":
        fail("journal contract status drifted")
    if value.get("commands") != ["init", "status", "append", "export"]:
        fail("journal command set or order drifted")
    authority = verifier.require_object(value.get("authority"), "journal_contract.authority")
    if authority.get("capabilities_granted") != [] or authority.get("mutation_capabilities") != []:
        fail("journal contract grants authority")
    return value


def journal_digest(value: dict[str, Any]) -> str:
    unsigned = dict(value)
    unsigned.pop("journal_digest", None)
    return sha256_bytes(canonical_json(unsigned))


def expected_case_list(contract: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    return verifier.expected_cases(contract)


def normalized_source(native: dict[str, Any], native_file_sha256: str) -> dict[str, Any]:
    return {
        "dfmcp_commit": native["source"]["dfmcp_commit"],
        "dfmcp_dirty": False,
        "dfhack_commit": native["source"]["dfhack_commit"],
        "dfhack_dirty": False,
        "plugin_sha256": native["plugin"]["sha256"],
        "native_build_receipt_sha256": native_file_sha256,
    }


def validate_version_tuple(value: dict[str, Any]) -> dict[str, Any]:
    verifier.require_exact_keys(
        value,
        {"dwarf_fortress", "dfhack", "bridge", "protocol"},
        "journal.version_tuple",
    )
    normalized = {
        "dwarf_fortress": verifier.require_string(
            value.get("dwarf_fortress"), "journal.version_tuple.dwarf_fortress", 128
        ),
        "dfhack": verifier.require_string(
            value.get("dfhack"), "journal.version_tuple.dfhack", 128
        ),
        "bridge": verifier.require_string(
            value.get("bridge"), "journal.version_tuple.bridge", 64
        ),
        "protocol": verifier.require_string(
            value.get("protocol"), "journal.version_tuple.protocol", 16
        ),
    }
    if normalized["bridge"] != "0.2.0" or normalized["protocol"] != "1.1":
        fail("journal accepts only bridge 0.2.0 protocol 1.1")
    return normalized


def validate_host(value: dict[str, Any]) -> dict[str, Any]:
    verifier.require_exact_keys(value, {"system", "machine"}, "journal.host")
    return {
        "system": verifier.require_string(value.get("system"), "journal.host.system", 128),
        "machine": verifier.require_string(value.get("machine"), "journal.host.machine", 128),
    }


def validate_journal(
    value: dict[str, Any],
    acceptance_contract: dict[str, Any],
    acceptance_contract_sha256: str,
    native: dict[str, Any],
    native_file_sha256: str,
) -> list[dict[str, Any]]:
    verifier.require_exact_keys(
        value,
        {
            "schema",
            "status",
            "acceptance_contract_sha256",
            "native_build_receipt_sha256",
            "source",
            "version_tuple",
            "host",
            "next_sequence",
            "events",
            "journal_digest",
        },
        "journal",
    )
    if value.get("schema") != JOURNAL_SCHEMA:
        fail("journal schema is unsupported")
    if value.get("status") not in {"capturing", "complete"}:
        fail("journal status is unsupported")
    if verifier.require_hash(
        value.get("acceptance_contract_sha256"),
        "journal.acceptance_contract_sha256",
    ) != acceptance_contract_sha256:
        fail("journal is bound to different acceptance contract bytes")
    if verifier.require_hash(
        value.get("native_build_receipt_sha256"),
        "journal.native_build_receipt_sha256",
    ) != native_file_sha256:
        fail("journal is bound to different native receipt bytes")
    if value.get("source") != normalized_source(native, native_file_sha256):
        fail("journal source identity differs from the native receipt")
    version = validate_version_tuple(
        verifier.require_object(value.get("version_tuple"), "journal.version_tuple")
    )
    host = validate_host(verifier.require_object(value.get("host"), "journal.host"))
    events = verifier.require_list(value.get("events"), "journal.events")
    cases = expected_case_list(acceptance_contract)
    if len(events) > len(cases):
        fail("journal contains more events than the acceptance contract")
    expected_next = len(events) + 1
    if value.get("next_sequence") != expected_next:
        fail(f"journal next_sequence must be {expected_next}")
    expected_status = "complete" if len(events) == len(cases) else "capturing"
    if value.get("status") != expected_status:
        fail(f"journal status must be {expected_status}")
    if verifier.require_hash(value.get("journal_digest"), "journal.journal_digest") != journal_digest(value):
        fail("journal digest is not canonical")

    normalized_events: list[dict[str, Any]] = []
    source = normalized_source(native, native_file_sha256)
    for index, event in enumerate(events, 1):
        gate, case = cases[index - 1]
        verifier.require_exact_keys(
            verifier.require_object(event, f"journal.events[{index}]") ,
            set(acceptance_contract["event_fields"]),
            f"journal.events[{index}]",
        )
        if event.get("schema") != acceptance_contract["event_schema"]:
            fail(f"journal event {index} schema drifted")
        if event.get("sequence") != index or event.get("gate") != gate or event.get("case") != case["case"]:
            fail(f"journal event {index} is out of contract order")
        if event.get("status") != "passed":
            fail(f"journal event {index} is not passed evidence")
        if event.get("source") != source or event.get("version_tuple") != version or event.get("host") != host:
            fail(f"journal event {index} crosses source, version, or platform identity")
        assertions = verifier.require_object(
            event.get("assertions"), f"journal.events[{index}].assertions"
        )
        if assertions != case["required_equals"]:
            fail(f"journal event {index} assertions drifted")
        artifacts = verifier.require_object(
            event.get("artifacts"), f"journal.events[{index}].artifacts"
        )
        if list(artifacts) != case["required_artifact_digests"]:
            fail(f"journal event {index} artifact order drifted")
        for name, digest in artifacts.items():
            require_hash(str(digest), f"journal.events[{index}].artifacts.{name}")
        evidence_digest = sha256_bytes(
            canonical_json({"assertions": assertions, "artifacts": artifacts})
        )
        if event.get("evidence_digest") != evidence_digest:
            fail(f"journal event {index} evidence digest drifted")
        normalized_events.append(event)
    return normalized_events


def private_atomic_write(path: Path, value: dict[str, Any]) -> None:
    ensure_path_bound(path, "journal")
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    parent_metadata = path.parent.stat()
    if not stat.S_ISDIR(parent_metadata.st_mode):
        fail("journal parent is not a directory")
    if hasattr(os, "geteuid") and parent_metadata.st_uid not in {0, os.geteuid()}:
        fail("journal parent is not owned by root or the effective user")
    payload = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    if len(payload.encode("utf-8")) > MAX_JOURNAL_BYTES:
        fail("journal exceeds its byte bound")
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise


@contextmanager
def journal_lock(path: Path) -> Iterator[None]:
    lock = path.with_name(f".{path.name}.lock")
    ensure_path_bound(lock, "journal lock")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_CLOEXEC"):
        flags |= os.O_CLOEXEC
    try:
        descriptor = os.open(lock, flags, 0o600)
    except FileExistsError:
        fail("announcement evidence journal lock already exists")
    try:
        os.write(descriptor, f"pid={os.getpid()}\n".encode())
        os.fsync(descriptor)
        yield
    finally:
        os.close(descriptor)
        try:
            os.unlink(lock)
        except FileNotFoundError:
            pass


def read_assertions(path: Path) -> dict[str, Any]:
    raw, _ = stable_regular_bytes(path, MAX_ASSERTIONS_BYTES, "assertions file")
    return verifier.parse_object(raw, "assertions file")


def parse_artifact_argument(raw: str) -> tuple[str, str]:
    name, separator, value = raw.partition("=")
    if not separator or not name or not value:
        fail("artifact arguments must have form name=value")
    if any(character.isspace() or character.iscontrol() for character in name):
        fail("artifact name contains whitespace or a control character")
    return name, value


def resolve_artifacts(
    required_names: list[str],
    artifact_paths: list[str],
    artifact_hashes: list[str],
) -> dict[str, str]:
    supplied: dict[str, str] = {}
    for raw in artifact_paths:
        name, value = parse_artifact_argument(raw)
        if name in supplied:
            fail(f"artifact {name!r} was supplied more than once")
        supplied[name] = stable_file_sha256(
            Path(value), MAX_ARTIFACT_BYTES, f"artifact {name}"
        )
    for raw in artifact_hashes:
        name, value = parse_artifact_argument(raw)
        if name in supplied:
            fail(f"artifact {name!r} was supplied more than once")
        supplied[name] = require_hash(value, f"artifact {name}")
    if list(supplied) != required_names:
        fail(
            f"artifact order differs: expected {required_names}, got {list(supplied)}"
        )
    return supplied


def initialize(
    journal_path: Path,
    native_receipt_path: Path,
    acceptance_path: Path,
    journal_contract_path: Path,
    df_version: str,
    dfhack_version: str,
    system: str,
    machine: str,
) -> dict[str, Any]:
    if journal_path.exists() or journal_path.is_symlink():
        fail("journal already exists; initialization is exclusive")
    journal_contract = load_journal_contract(journal_contract_path)
    expected_acceptance = journal_contract["acceptance_contract"]
    try:
        relative_acceptance = acceptance_path.resolve().relative_to(ROOT.resolve()).as_posix()
    except ValueError:
        relative_acceptance = ""
    if relative_acceptance and relative_acceptance != expected_acceptance:
        fail("journal contract names a different acceptance contract path")
    acceptance, acceptance_sha = validate_contract_file(
        acceptance_path, 4 * 1024 * 1024, "acceptance contract"
    )
    acceptance = verifier.load_contract(acceptance_path)
    native, native_sha = verifier.validate_native_receipt(native_receipt_path)
    value: dict[str, Any] = {
        "schema": JOURNAL_SCHEMA,
        "status": "capturing",
        "acceptance_contract_sha256": acceptance_sha,
        "native_build_receipt_sha256": native_sha,
        "source": normalized_source(native, native_sha),
        "version_tuple": validate_version_tuple(
            {
                "dwarf_fortress": df_version,
                "dfhack": dfhack_version,
                "bridge": "0.2.0",
                "protocol": "1.1",
            }
        ),
        "host": validate_host({"system": system, "machine": machine}),
        "next_sequence": 1,
        "events": [],
    }
    value["journal_digest"] = journal_digest(value)
    validate_journal(value, acceptance, acceptance_sha, native, native_sha)
    private_atomic_write(journal_path, value)
    return value


def append_event(
    journal_path: Path,
    native_receipt_path: Path,
    acceptance_path: Path,
    expected_journal_sha256: str,
    assertions_path: Path,
    artifact_paths: list[str],
    artifact_hashes: list[str],
) -> dict[str, Any]:
    require_hash(expected_journal_sha256, "expected_journal_sha256")
    with journal_lock(journal_path):
        acceptance, acceptance_sha = validate_contract_file(
            acceptance_path, 4 * 1024 * 1024, "acceptance contract"
        )
        acceptance = verifier.load_contract(acceptance_path)
        native, native_sha = verifier.validate_native_receipt(native_receipt_path)
        journal, actual_sha = read_private_object(journal_path, "announcement evidence journal")
        if actual_sha != expected_journal_sha256:
            fail(
                f"journal compare-and-swap fence failed: expected {expected_journal_sha256}, got {actual_sha}"
            )
        events = validate_journal(
            journal, acceptance, acceptance_sha, native, native_sha
        )
        cases = expected_case_list(acceptance)
        if len(events) == len(cases):
            fail("announcement evidence journal is already complete")
        gate, case = cases[len(events)]
        assertions = read_assertions(assertions_path)
        if assertions != case["required_equals"]:
            fail(
                f"assertions do not satisfy next case {gate}/{case['case']}"
            )
        artifacts = resolve_artifacts(
            list(case["required_artifact_digests"]), artifact_paths, artifact_hashes
        )
        sequence = len(events) + 1
        event = {
            "schema": acceptance["event_schema"],
            "sequence": sequence,
            "gate": gate,
            "case": case["case"],
            "status": "passed",
            "source": journal["source"],
            "version_tuple": journal["version_tuple"],
            "host": journal["host"],
            "assertions": assertions,
            "artifacts": artifacts,
            "evidence_digest": sha256_bytes(
                canonical_json({"assertions": assertions, "artifacts": artifacts})
            ),
        }
        updated = dict(journal)
        updated_events = [*events, event]
        updated["events"] = updated_events
        updated["next_sequence"] = len(updated_events) + 1
        updated["status"] = (
            "complete" if len(updated_events) == len(cases) else "capturing"
        )
        updated["journal_digest"] = journal_digest(updated)
        validate_journal(updated, acceptance, acceptance_sha, native, native_sha)
        private_atomic_write(journal_path, updated)
        return updated


def export_stream(
    journal_path: Path,
    native_receipt_path: Path,
    acceptance_path: Path,
    expected_journal_sha256: str,
    output_path: Path,
) -> dict[str, Any]:
    require_hash(expected_journal_sha256, "expected_journal_sha256")
    with journal_lock(journal_path):
        acceptance, acceptance_sha = validate_contract_file(
            acceptance_path, 4 * 1024 * 1024, "acceptance contract"
        )
        acceptance = verifier.load_contract(acceptance_path)
        native, native_sha = verifier.validate_native_receipt(native_receipt_path)
        journal, actual_sha = read_private_object(journal_path, "announcement evidence journal")
        if actual_sha != expected_journal_sha256:
            fail(
                f"journal compare-and-swap fence failed: expected {expected_journal_sha256}, got {actual_sha}"
            )
        events = validate_journal(
            journal, acceptance, acceptance_sha, native, native_sha
        )
        if journal["status"] != "complete":
            fail("incomplete announcement evidence journals cannot be exported")
        payload = b"".join(canonical_json(event) + b"\n" for event in events)
        if output_path.exists() or output_path.is_symlink():
            fail("event stream output already exists; export is create-only")
        output_path.parent.mkdir(parents=True, exist_ok=True)
        descriptor = os.open(
            output_path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0),
            0o600,
        )
        try:
            os.write(descriptor, payload)
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        directory = os.open(output_path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        receipt = verifier.verify(output_path, native_receipt_path, acceptance_path)
        return {
            "output": os.fspath(output_path),
            "stream_sha256": sha256_bytes(payload),
            "event_count": len(events),
            "acceptance_receipt_digest": receipt["receipt_digest"],
        }


def next_case_status(
    journal_path: Path,
    native_receipt_path: Path,
    acceptance_path: Path,
) -> dict[str, Any]:
    acceptance, acceptance_sha = validate_contract_file(
        acceptance_path, 4 * 1024 * 1024, "acceptance contract"
    )
    acceptance = verifier.load_contract(acceptance_path)
    native, native_sha = verifier.validate_native_receipt(native_receipt_path)
    journal, journal_sha = read_private_object(journal_path, "announcement evidence journal")
    events = validate_journal(journal, acceptance, acceptance_sha, native, native_sha)
    cases = expected_case_list(acceptance)
    next_case: dict[str, Any] | None = None
    if len(events) < len(cases):
        gate, case = cases[len(events)]
        next_case = {
            "sequence": len(events) + 1,
            "gate": gate,
            "case": case["case"],
            "required_assertions": case["required_equals"],
            "required_artifacts": case["required_artifact_digests"],
        }
    return {
        "schema": JOURNAL_SCHEMA,
        "status": journal["status"],
        "journal_file_sha256": journal_sha,
        "journal_digest": journal["journal_digest"],
        "captured_events": len(events),
        "total_events": len(cases),
        "next_case": next_case,
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    init = subparsers.add_parser("init")
    init.add_argument("journal", type=Path)
    init.add_argument("--native-receipt", type=Path, required=True)
    init.add_argument("--acceptance-contract", type=Path, default=DEFAULT_ACCEPTANCE)
    init.add_argument("--journal-contract", type=Path, default=DEFAULT_JOURNAL_CONTRACT)
    init.add_argument("--df-version", required=True)
    init.add_argument("--dfhack-version", required=True)
    init.add_argument("--system", default=platform.system())
    init.add_argument("--machine", default=platform.machine())

    status = subparsers.add_parser("status")
    status.add_argument("journal", type=Path)
    status.add_argument("--native-receipt", type=Path, required=True)
    status.add_argument("--acceptance-contract", type=Path, default=DEFAULT_ACCEPTANCE)

    append = subparsers.add_parser("append")
    append.add_argument("journal", type=Path)
    append.add_argument("--native-receipt", type=Path, required=True)
    append.add_argument("--acceptance-contract", type=Path, default=DEFAULT_ACCEPTANCE)
    append.add_argument("--expected-journal-sha256", required=True)
    append.add_argument("--assertions", type=Path, required=True)
    append.add_argument("--artifact", action="append", default=[])
    append.add_argument("--artifact-sha256", action="append", default=[])

    export = subparsers.add_parser("export")
    export.add_argument("journal", type=Path)
    export.add_argument("output", type=Path)
    export.add_argument("--native-receipt", type=Path, required=True)
    export.add_argument("--acceptance-contract", type=Path, default=DEFAULT_ACCEPTANCE)
    export.add_argument("--expected-journal-sha256", required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.command == "init":
            value = initialize(
                args.journal,
                args.native_receipt,
                args.acceptance_contract,
                args.journal_contract,
                args.df_version,
                args.dfhack_version,
                args.system,
                args.machine,
            )
            result = {
                "status": value["status"],
                "journal_digest": value["journal_digest"],
                "next_sequence": value["next_sequence"],
            }
        elif args.command == "status":
            result = next_case_status(
                args.journal, args.native_receipt, args.acceptance_contract
            )
        elif args.command == "append":
            value = append_event(
                args.journal,
                args.native_receipt,
                args.acceptance_contract,
                args.expected_journal_sha256,
                args.assertions,
                args.artifact,
                args.artifact_sha256,
            )
            result = {
                "status": value["status"],
                "journal_digest": value["journal_digest"],
                "captured_events": len(value["events"]),
                "next_sequence": value["next_sequence"],
            }
        else:
            result = export_stream(
                args.journal,
                args.native_receipt,
                args.acceptance_contract,
                args.expected_journal_sha256,
                args.output,
            )
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    except (
        OSError,
        verifier.VerificationError,
        verifier.promotion.PromotionError,
        JournalError,
    ) as exc:
        print(f"live announcement evidence journal: FAIL: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
