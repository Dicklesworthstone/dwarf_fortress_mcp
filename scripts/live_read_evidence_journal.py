#!/usr/bin/env python3
"""Capture and finalize one ordered, source-bound R2-R5 evidence journal."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import verify_live_read_acceptance as verifier

JOURNAL_SCHEMA = "dfmcp.live-read-evidence-journal/1"
PROBE_SCHEMA = "dfmcp.live-read-probe/1"
STATE_FILE = "journal.json"
EVENTS_DIRECTORY = "events"
RAW_DIRECTORY = "raw"
NATIVE_RECEIPT_FILE = "native-build-receipt.json"
EVIDENCE_FILE = "evidence.jsonl"
ACCEPTANCE_RECEIPT_FILE = "live-read-acceptance-receipt.json"
CHECKSUM_FILE = "SHA256SUMS"
MAX_JOURNAL_BYTES = 4 * 1024 * 1024


class JournalError(ValueError):
    pass


def fail(message: str) -> None:
    raise JournalError(message)


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(
        "utf-8"
    )


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_write_bytes(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    atomic_write_bytes(
        path,
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8") + b"\n",
    )


def read_json(path: Path, maximum: int, label: str) -> dict[str, Any]:
    if path.is_symlink():
        fail(f"{label} must not be a symbolic link")
    try:
        size = path.stat().st_size
    except OSError as exc:
        fail(f"cannot stat {label}: {exc}")
    if size <= 0 or size > maximum:
        fail(f"{label} must contain 1..={maximum} bytes, got {size}")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse {label}: {exc}")
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def git_output(arguments: list[str]) -> str:
    try:
        completed = subprocess.run(
            ["git", "-C", str(ROOT), *arguments],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"git command failed: {exc}")
    return completed.stdout.strip()


def current_source() -> tuple[str, bool]:
    commit = git_output(["rev-parse", "HEAD"])
    if len(commit) != 40 or any(character not in "0123456789abcdef" for character in commit):
        fail("current Git revision is not a canonical lowercase commit ID")
    dirty = bool(git_output(["status", "--porcelain=v1"]))
    return commit, dirty


def contract() -> dict[str, Any]:
    return verifier.load_contract(ROOT / "architecture/live_read_acceptance_v1.json")


def expected_sequence(value: dict[str, Any]) -> list[tuple[str, str]]:
    return verifier.expected_sequence(value)


def event_filename(index: int, gate: str, case: str) -> str:
    safe_gate = "".join(character for character in gate.lower() if character.isalnum() or character == "-")
    safe_case = "".join(
        character for character in case.lower() if character.isalnum() or character in "-_"
    )
    if not safe_gate or not safe_case:
        fail("gate and case do not form a safe evidence filename")
    return f"{index:03d}-{safe_gate}-{safe_case}.json"


def validate_native_receipt(path: Path, commit: str) -> tuple[dict[str, Any], bytes]:
    if path.is_symlink():
        fail("native build receipt must not be a symbolic link")
    try:
        raw = path.read_bytes()
    except OSError as exc:
        fail(f"cannot read native build receipt: {exc}")
    if not raw or len(raw) > 4 * 1024 * 1024:
        fail("native build receipt violates its 4 MiB bound")
    try:
        receipt = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse native build receipt: {exc}")
    if not isinstance(receipt, dict):
        fail("native build receipt must be an object")
    if receipt.get("schema") != "dfmcp.dfhack-plugin-qualification/1":
        fail("native build receipt schema is unsupported")
    if receipt.get("status") != "native-build-passed":
        fail("native build receipt is not passing R1 evidence")
    source = receipt.get("source")
    plugin = receipt.get("plugin")
    if not isinstance(source, dict) or not isinstance(plugin, dict):
        fail("native build receipt omits source or plugin identity")
    if source.get("dfmcp_commit") != commit:
        fail("native build receipt is not bound to the current dfmcp commit")
    if source.get("dfmcp_dirty") is not False:
        fail("native build receipt is not bound to a clean source revision")
    if plugin.get("rpc_methods") != ["Handshake", "ReadObservation"]:
        fail("native build receipt method set drifted")
    if plugin.get("mutation_rpc_methods") != []:
        fail("native build receipt contains mutation methods")
    verifier.require_commit(source.get("dfhack_commit"), "native.source.dfhack_commit")
    verifier.require_hash(plugin.get("sha256"), "native.plugin.sha256")
    return receipt, raw


def source_digests(value: dict[str, Any]) -> dict[str, str]:
    bindings = value["source_binding"]["required_source_digests"]
    output: dict[str, str] = {}
    for name, relative in bindings.items():
        path = ROOT / relative
        if not path.is_file() or path.is_symlink():
            fail(f"required source binding is missing or redirected: {relative}")
        output[name] = digest_file(path)
    return output


def validate_version(value: str, label: str) -> str:
    encoded = value.encode("utf-8")
    if not encoded or len(encoded) > 128 or any(ord(character) < 0x20 for character in value):
        fail(f"{label} must contain 1..=128 printable UTF-8 bytes")
    return value


def initialize(args: argparse.Namespace) -> dict[str, Any]:
    run_directory = args.run_directory.resolve()
    if run_directory.is_symlink():
        fail("run directory must not be a symbolic link")
    if run_directory.exists() and any(run_directory.iterdir()):
        fail("run directory already exists and is not empty")
    run_directory.mkdir(parents=True, exist_ok=True)
    (run_directory / EVENTS_DIRECTORY).mkdir()
    (run_directory / RAW_DIRECTORY).mkdir()

    value = contract()
    commit, dirty = current_source()
    if dirty and not args.allow_dirty_development:
        fail("live evidence capture requires a clean source tree")
    native, native_raw = validate_native_receipt(args.native_build_receipt, commit)
    native_copy = run_directory / NATIVE_RECEIPT_FILE
    atomic_write_bytes(native_copy, native_raw)

    source = native["source"]
    plugin = native["plugin"]
    run_id = args.run_id or f"live-read-{commit[:12]}-{source['dfhack_commit'][:12]}"
    if not verifier.EVENT_ID.fullmatch(run_id):
        fail("run_id must match the bounded event-identity alphabet")
    manifest = {
        "schema": value["event_schema"],
        "event_id": "manifest",
        "gate": "meta",
        "case": "manifest",
        "result": "passed",
        "error_code": None,
        "run_id": run_id,
        "synthetic": False,
        "source": {
            "dfmcp_commit": commit,
            "dfmcp_dirty": dirty,
            "dfhack_commit": source["dfhack_commit"],
            "plugin_sha256": plugin["sha256"],
            "native_build_receipt_sha256": digest_bytes(native_raw),
            "dwarf_fortress_version": validate_version(
                args.dwarf_fortress_version, "Dwarf Fortress version"
            ),
            "dfhack_version": validate_version(args.dfhack_version, "DFHack version"),
            "bridge_version": validate_version(args.bridge_version, "bridge version"),
            "bridge_protocol": validate_version(args.bridge_protocol, "bridge protocol"),
        },
        "host": {
            "system": platform.system() or "unknown",
            "machine": platform.machine() or "unknown",
        },
        "source_digests": source_digests(value),
    }
    verifier.bounded_tree(manifest, value["limits"], "manifest")
    verifier.reject_secret_material(canonical_json(manifest), [manifest], value)
    event_name = event_filename(0, "meta", "manifest")
    event_content = canonical_json(manifest) + b"\n"
    atomic_write_bytes(run_directory / EVENTS_DIRECTORY / event_name, event_content)
    state = {
        "schema": JOURNAL_SCHEMA,
        "sealed": False,
        "development_evidence": dirty,
        "contract_sha256": digest_file(ROOT / "architecture/live_read_acceptance_v1.json"),
        "source_commit": commit,
        "native_build_receipt_sha256": digest_bytes(native_raw),
        "next_index": 1,
        "records": [
            {
                "index": 0,
                "gate": "meta",
                "case": "manifest",
                "event_id": "manifest",
                "event_file": f"{EVENTS_DIRECTORY}/{event_name}",
                "event_sha256": digest_bytes(event_content),
                "raw_file": None,
                "raw_sha256": None,
            }
        ],
    }
    atomic_write_json(run_directory / STATE_FILE, state)
    return status_payload(run_directory, state, value)


def load_journal(run_directory: Path) -> tuple[Path, dict[str, Any], dict[str, Any]]:
    resolved = run_directory.resolve()
    if resolved.is_symlink() or not resolved.is_dir():
        fail("run directory is missing or redirected")
    value = contract()
    state = read_json(resolved / STATE_FILE, MAX_JOURNAL_BYTES, "journal state")
    if state.get("schema") != JOURNAL_SCHEMA:
        fail("journal schema is unsupported")
    if state.get("contract_sha256") != digest_file(
        ROOT / "architecture/live_read_acceptance_v1.json"
    ):
        fail("journal was created for a different acceptance contract")
    records = state.get("records")
    if not isinstance(records, list) or len(records) != state.get("next_index"):
        fail("journal record count and next index disagree")
    sequence = expected_sequence(value)
    if len(records) > len(sequence):
        fail("journal contains more events than the acceptance contract")
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            fail(f"journal record {index} is not an object")
        if record.get("index") != index:
            fail(f"journal record {index} has the wrong index")
        if (record.get("gate"), record.get("case")) != sequence[index]:
            fail(f"journal record {index} violates the normative event order")
        event_path = resolved / str(record.get("event_file"))
        if event_path.is_symlink() or not event_path.is_file():
            fail(f"journal event {index} is missing or redirected")
        if digest_file(event_path) != record.get("event_sha256"):
            fail(f"journal event {index} digest mismatch")
        raw_file = record.get("raw_file")
        if raw_file is not None:
            raw_path = resolved / str(raw_file)
            if raw_path.is_symlink() or not raw_path.is_file():
                fail(f"journal raw artifact {index} is missing or redirected")
            if digest_file(raw_path) != record.get("raw_sha256"):
                fail(f"journal raw artifact {index} digest mismatch")
    return resolved, state, value


def expected_case(value: dict[str, Any], index: int) -> tuple[str, str] | None:
    sequence = expected_sequence(value)
    return sequence[index] if index < len(sequence) else None


def contract_case(value: dict[str, Any], gate: str, case: str) -> dict[str, Any]:
    for item in value["gates"][gate]["required_cases"]:
        if item["case"] == case:
            return item
    fail(f"acceptance contract does not contain {gate}/{case}")


def event_base(value: dict[str, Any], gate: str, case: str) -> dict[str, Any]:
    expected = contract_case(value, gate, case)
    return {
        "schema": value["event_schema"],
        "event_id": f"{gate.lower()}.{case}",
        "gate": gate,
        "case": case,
        "result": expected["result"],
        "error_code": expected["error_code"],
    }


def require_probe(value: dict[str, Any], kind: str) -> None:
    if value.get("schema") != PROBE_SCHEMA or value.get("kind") != kind:
        fail(f"probe artifact must be schema {PROBE_SCHEMA!r} and kind {kind!r}")


def normalize_probe(
    value: dict[str, Any], gate: str, case: str, acceptance: dict[str, Any]
) -> dict[str, Any]:
    event = event_base(acceptance, gate, case)
    expected = contract_case(acceptance, gate, case)
    if gate == "R2":
        if case == "secret_scan":
            fail("R2/secret_scan requires a normalized scanner event, not a bridge probe")
        require_probe(value, "handshake")
        if value.get("case") != case:
            fail(f"probe case {value.get('case')!r} does not match R2/{case}")
        if bool(value.get("accepted")) != (expected["result"] == "accepted"):
            fail(f"R2/{case} probe acceptance disagrees with the contract")
        if value.get("error_code") != expected["error_code"]:
            fail(f"R2/{case} probe error code disagrees with the contract")
        for name in [
            "sensitive_manifest_disclosed",
            "bridge_generation",
            "supported_methods",
            "world_loaded",
            "fortress_mode",
            "nonce_correlated",
            "protocol_major",
            "protocol_minor",
            "bridge_version",
            "dfhack_version",
            "dwarf_fortress_version",
        ]:
            event[name] = value.get(name)
        return event

    capsule_cases = {
        "baseline_names_included",
        "repeat_names_included",
        "page_size_1",
        "page_size_2",
        "page_size_7",
        "page_size_64",
        "page_size_256",
        "page_size_4096",
        "baseline_names_omitted",
        "repeat_names_omitted",
    }
    if gate == "R3" and case in capsule_cases:
        require_probe(value, "capsule")
        for name in [
            "paused",
            "names_included",
            "page_size",
            "page_count",
            "citizen_count",
            "complete",
            "publication_count",
            "bridge_generation",
            "capsule_sha256",
            "snapshot_sha256",
            "receipt_sha256",
            "citizen_identity_sha256",
            "anchor",
        ]:
            event[name] = value.get(name)
        if case.startswith("page_size_"):
            required_size = int(case.removeprefix("page_size_"))
            if event["page_size"] != required_size:
                fail(f"R3/{case} probe did not use page size {required_size}")
        if case.endswith("names_included") and event["names_included"] is not True:
            fail(f"R3/{case} did not include names")
        if case.endswith("names_omitted") and event["names_included"] is not False:
            fail(f"R3/{case} did not omit names")
        return event

    observation_cases = {
        "offset_at_total",
        "offset_beyond_total",
        "oversize_request",
        "running_multipage_rejected",
    }
    if gate == "R3" and case in observation_cases:
        require_probe(value, "observation")
        if value.get("case") != case:
            fail(f"probe case {value.get('case')!r} does not match R3/{case}")
        if value.get("error_code") != expected["error_code"]:
            fail(f"R3/{case} probe error code disagrees with the contract")
        for name in [
            "citizen_count",
            "requested_offset",
            "canonical_offset",
            "returned_citizens",
            "complete",
            "paused",
            "pages_attempted",
            "published",
        ]:
            event[name] = value.get(name)
        return event

    if gate == "R4" and case in {"world_unloaded", "non_fortress_mode"}:
        require_probe(value, "observation")
        if value.get("case") != case or value.get("error_code") != expected["error_code"]:
            fail(f"R4/{case} probe identity or error code disagrees with the contract")
        event["published"] = value.get("published")
        event["world_loaded"] = value.get("world_loaded")
        event["fortress_mode"] = value.get("fortress_mode")
        return event

    if gate == "R4" and case == "fresh_handshake":
        require_probe(value, "handshake")
        if value.get("case") != "correct_token" or value.get("accepted") is not True:
            fail("R4/fresh_handshake requires a successful correct_token probe")
        event["bridge_generation"] = value.get("bridge_generation")
        event["supported_methods"] = value.get("supported_methods")
        return event

    fail(f"{gate}/{case} requires a composite or normalized event rather than one probe artifact")


def validate_event(
    event: dict[str, Any], value: dict[str, Any], gate: str, case: str
) -> None:
    if event.get("schema") != value["event_schema"]:
        fail("event schema drifted")
    if event.get("gate") != gate or event.get("case") != case:
        fail("event gate/case does not match the next journal slot")
    expected = contract_case(value, gate, case)
    if event.get("result") != expected["result"] or event.get("error_code") != expected["error_code"]:
        fail("event result or error code disagrees with the acceptance contract")
    event_id = event.get("event_id")
    if not isinstance(event_id, str) or verifier.EVENT_ID.fullmatch(event_id) is None:
        fail("event_id violates the bounded identity alphabet")
    verifier.bounded_tree(event, value["limits"], f"{gate}/{case}")
    verifier.reject_secret_material(canonical_json(event), [event], value)


def append_record(
    run_directory: Path,
    state: dict[str, Any],
    value: dict[str, Any],
    event: dict[str, Any],
    raw: bytes,
) -> dict[str, Any]:
    if state.get("sealed") is True:
        fail("sealed evidence journal cannot be modified")
    index = state["next_index"]
    expected = expected_case(value, index)
    if expected is None:
        fail("all acceptance events are already recorded")
    gate, case = expected
    validate_event(event, value, gate, case)
    event_name = event_filename(index, gate, case)
    raw_name = f"{index:03d}-{gate.lower()}-{case}-raw.json"
    event_path = run_directory / EVENTS_DIRECTORY / event_name
    raw_path = run_directory / RAW_DIRECTORY / raw_name
    event_content = canonical_json(event) + b"\n"
    raw_content = raw if raw.endswith(b"\n") else raw + b"\n"
    if event_path.exists() and digest_file(event_path) != digest_bytes(event_content):
        fail("orphan event file conflicts with the event being appended")
    if raw_path.exists() and digest_file(raw_path) != digest_bytes(raw_content):
        fail("orphan raw artifact conflicts with the artifact being appended")
    if not raw_path.exists():
        atomic_write_bytes(raw_path, raw_content)
    if not event_path.exists():
        atomic_write_bytes(event_path, event_content)
    state["records"].append(
        {
            "index": index,
            "gate": gate,
            "case": case,
            "event_id": event["event_id"],
            "event_file": f"{EVENTS_DIRECTORY}/{event_name}",
            "event_sha256": digest_bytes(event_content),
            "raw_file": f"{RAW_DIRECTORY}/{raw_name}",
            "raw_sha256": digest_bytes(raw_content),
        }
    )
    state["next_index"] = index + 1
    atomic_write_json(run_directory / STATE_FILE, state)
    return status_payload(run_directory, state, value)


def append_event(args: argparse.Namespace, from_probe: bool) -> dict[str, Any]:
    run_directory, state, value = load_journal(args.run_directory)
    expected = expected_case(value, state["next_index"])
    if expected is None:
        fail("journal already contains the complete acceptance sequence")
    gate, case = expected
    raw_path = args.input
    if raw_path.is_symlink():
        fail("input artifact must not be a symbolic link")
    try:
        raw = raw_path.read_bytes()
    except OSError as exc:
        fail(f"cannot read input artifact: {exc}")
    if not raw or len(raw) > value["limits"]["maximum_event_bytes"]:
        fail("input artifact violates the event byte bound")
    try:
        input_value = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse input artifact: {exc}")
    if not isinstance(input_value, dict):
        fail("input artifact must be a JSON object")
    event = normalize_probe(input_value, gate, case, value) if from_probe else input_value
    return append_record(run_directory, state, value, event, raw)


def status_payload(
    run_directory: Path, state: dict[str, Any], value: dict[str, Any]
) -> dict[str, Any]:
    sequence = expected_sequence(value)
    next_item = expected_case(value, state["next_index"])
    return {
        "schema": JOURNAL_SCHEMA,
        "run_directory": str(run_directory),
        "sealed": state.get("sealed") is True,
        "development_evidence": state.get("development_evidence") is True,
        "recorded_events": len(state["records"]),
        "required_events": len(sequence),
        "next": None if next_item is None else {"gate": next_item[0], "case": next_item[1]},
        "source_commit": state["source_commit"],
    }


def show_status(args: argparse.Namespace) -> dict[str, Any]:
    run_directory, state, value = load_journal(args.run_directory)
    return status_payload(run_directory, state, value)


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    run_directory, state, value = load_journal(args.run_directory)
    sequence = expected_sequence(value)
    if len(state["records"]) != len(sequence):
        next_item = expected_case(value, state["next_index"])
        fail(f"journal is incomplete; next required event is {next_item}")
    events: list[dict[str, Any]] = []
    lines: list[bytes] = []
    for record in state["records"]:
        event_path = run_directory / record["event_file"]
        event = read_json(event_path, value["limits"]["maximum_event_bytes"], "journal event")
        events.append(event)
        lines.append(canonical_json(event) + b"\n")
    verifier.reject_secret_material(b"".join(lines), events, value)
    verifier.validate_event_sequence(events, value)
    evidence_path = run_directory / EVIDENCE_FILE
    atomic_write_bytes(evidence_path, b"".join(lines))
    receipt = verifier.verify_acceptance(
        evidence_path,
        ROOT / "architecture/live_read_acceptance_v1.json",
        verifier.VerificationOptions(
            source_root=ROOT,
            expected_dfmcp_commit=state["source_commit"],
            native_build_receipt=run_directory / NATIVE_RECEIPT_FILE,
            allow_dirty_development=state.get("development_evidence") is True,
        ),
    )
    receipt_path = run_directory / ACCEPTANCE_RECEIPT_FILE
    verifier.write_atomic(receipt_path, receipt)
    checksum_entries = []
    for name in [EVIDENCE_FILE, NATIVE_RECEIPT_FILE, ACCEPTANCE_RECEIPT_FILE]:
        checksum_entries.append(f"{digest_file(run_directory / name)}  {name}\n")
    atomic_write_bytes(
        run_directory / CHECKSUM_FILE,
        "".join(checksum_entries).encode("ascii"),
    )
    state["sealed"] = True
    state["evidence_sha256"] = digest_file(evidence_path)
    state["acceptance_receipt_sha256"] = digest_file(receipt_path)
    state["receipt_digest"] = receipt["receipt_digest"]
    atomic_write_json(run_directory / STATE_FILE, state)
    return {
        **status_payload(run_directory, state, value),
        "evidence": str(evidence_path),
        "receipt": str(receipt_path),
        "checksums": str(run_directory / CHECKSUM_FILE),
        "receipt_status": receipt["status"],
        "receipt_digest": receipt["receipt_digest"],
    }


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    init = commands.add_parser("init", help="create a source-bound journal and manifest event")
    init.add_argument("run_directory", type=Path)
    init.add_argument("--native-build-receipt", type=Path, required=True)
    init.add_argument("--dwarf-fortress-version", required=True)
    init.add_argument("--dfhack-version", required=True)
    init.add_argument("--bridge-version", required=True)
    init.add_argument("--bridge-protocol", default="1.0")
    init.add_argument("--run-id")
    init.add_argument("--allow-dirty-development", action="store_true")

    append = commands.add_parser("append", help="append the exact next normalized event")
    append.add_argument("run_directory", type=Path)
    append.add_argument("input", type=Path)

    append_probe = commands.add_parser(
        "append-probe", help="normalize and append the exact next probe artifact"
    )
    append_probe.add_argument("run_directory", type=Path)
    append_probe.add_argument("input", type=Path)

    status = commands.add_parser("status", help="verify and report journal progress")
    status.add_argument("run_directory", type=Path)

    seal = commands.add_parser("finalize", help="verify all events and seal the acceptance receipt")
    seal.add_argument("run_directory", type=Path)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "init":
            result = initialize(args)
        elif args.command == "append":
            result = append_event(args, False)
        elif args.command == "append-probe":
            result = append_event(args, True)
        elif args.command == "status":
            result = show_status(args)
        elif args.command == "finalize":
            result = finalize(args)
        else:
            fail(f"unsupported command {args.command!r}")
    except (JournalError, verifier.VerificationError, OSError) as exc:
        print(f"live read evidence journal: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
