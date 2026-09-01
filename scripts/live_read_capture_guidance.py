#!/usr/bin/env python3
"""Return machine-readable guidance for the next live-read acceptance case."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
ROOT = SCRIPT_DIR.parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

import live_read_evidence_journal as journal

PLAN_PATH = ROOT / "architecture/live_read_capture_plan_v1.json"
PLAN_SCHEMA = "dfmcp.live-read-capture-plan/1"
MAX_PLAN_BYTES = 1024 * 1024
MAX_ARG_BYTES = 4096
MAX_PRECONDITIONS = 16
ALLOWED_CAPTURE_KINDS = {"probe", "scanner", "composite"}
ALLOWED_APPEND_MODES = {"append", "append-probe"}
FORBIDDEN_ARG_FRAGMENTS = {
    "DFMCP_BRIDGE_TOKEN=",
    "--token",
    "--secret",
    "RunCommand",
    "RunLua",
    "ApplyEffect",
    "SetPauseState",
}


class GuidanceError(ValueError):
    pass


def fail(message: str) -> None:
    raise GuidanceError(message)


def read_plan(path: Path = PLAN_PATH) -> dict[str, Any]:
    if path.is_symlink() or not path.is_file():
        fail("capture plan must be a real file")
    try:
        size = path.stat().st_size
    except OSError as exc:
        fail(f"cannot stat capture plan: {exc}")
    if size <= 0 or size > MAX_PLAN_BYTES:
        fail(f"capture plan must contain 1..={MAX_PLAN_BYTES} bytes")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"cannot parse capture plan: {exc}")
    if not isinstance(value, dict):
        fail("capture plan must be a JSON object")
    return value


def bounded_string(value: Any, path: str, maximum: int = MAX_ARG_BYTES) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > maximum:
        fail(f"{path} must contain 1..={maximum} UTF-8 bytes")
    if any(ord(character) < 0x20 for character in value):
        fail(f"{path} contains a control character")
    return value


def validate_argv(value: Any, path: str) -> list[str]:
    if not isinstance(value, list) or len(value) > 64:
        fail(f"{path} must be an argv array with at most 64 elements")
    output: list[str] = []
    for index, item in enumerate(value):
        argument = bounded_string(item, f"{path}[{index}]")
        if any(fragment in argument for fragment in FORBIDDEN_ARG_FRAGMENTS):
            fail(f"{path}[{index}] contains a forbidden authority or secret fragment")
        output.append(argument)
    if output and output[0] in {"bash", "sh", "zsh", "cmd", "powershell", "pwsh"}:
        fail(f"{path} must not route through a shell interpreter")
    if "-c" in output or "/c" in [argument.lower() for argument in output]:
        fail(f"{path} must not contain a shell-command argument")
    return output


def validate_plan(
    plan: dict[str, Any], acceptance: dict[str, Any]
) -> dict[tuple[str, str], dict[str, Any]]:
    if plan.get("schema_version") != PLAN_SCHEMA:
        fail("capture plan schema version is unsupported")
    contract = plan.get("output_contract")
    if not isinstance(contract, dict):
        fail("capture plan omits output_contract")
    for name in [
        "secrets_in_arguments",
        "arbitrary_dfhack_commands",
        "arbitrary_lua",
        "mutation_authority",
    ]:
        if contract.get(name) is not False:
            fail(f"capture plan output_contract.{name} must remain false")
    cases = plan.get("cases")
    if not isinstance(cases, list):
        fail("capture plan cases must be an array")
    expected = journal.expected_sequence(acceptance)[1:]
    actual: list[tuple[str, str]] = []
    indexed: dict[tuple[str, str], dict[str, Any]] = {}
    for index, raw in enumerate(cases):
        if not isinstance(raw, dict):
            fail(f"capture plan case {index} must be an object")
        gate = bounded_string(raw.get("gate"), f"cases[{index}].gate", 8)
        case = bounded_string(raw.get("case"), f"cases[{index}].case", 128)
        identity = (gate, case)
        if identity in indexed:
            fail(f"capture plan repeats {gate}/{case}")
        capture_kind = raw.get("capture_kind")
        if capture_kind not in ALLOWED_CAPTURE_KINDS:
            fail(f"capture plan {gate}/{case} has invalid capture_kind")
        if not isinstance(raw.get("automatable"), bool):
            fail(f"capture plan {gate}/{case} automatable must be Boolean")
        argv = validate_argv(raw.get("argv"), f"cases[{index}].argv")
        if capture_kind in {"probe", "scanner"} and not argv:
            fail(f"capture plan {gate}/{case} requires a nonempty argv")
        if capture_kind == "composite" and raw.get("automatable") is True:
            fail(f"composite case {gate}/{case} cannot be marked automatable")
        append_mode = raw.get("append_mode")
        if append_mode not in ALLOWED_APPEND_MODES:
            fail(f"capture plan {gate}/{case} has invalid append_mode")
        if capture_kind == "probe" and append_mode != "append-probe":
            fail(f"probe case {gate}/{case} must use append-probe")
        if capture_kind in {"scanner", "composite"} and append_mode != "append":
            fail(f"{capture_kind} case {gate}/{case} must use append")
        preconditions = raw.get("preconditions")
        if not isinstance(preconditions, list) or not preconditions or len(preconditions) > MAX_PRECONDITIONS:
            fail(f"capture plan {gate}/{case} requires 1..={MAX_PRECONDITIONS} preconditions")
        for precondition_index, precondition in enumerate(preconditions):
            bounded_string(
                precondition,
                f"cases[{index}].preconditions[{precondition_index}]",
            )
        bounded_string(raw.get("operator_action"), f"cases[{index}].operator_action")
        normalized = dict(raw)
        normalized["argv"] = argv
        indexed[identity] = normalized
        actual.append(identity)
    if actual != expected:
        fail(f"capture plan order/set differs from acceptance contract: expected {expected}, got {actual}")
    return indexed


def safe_artifact_name(index: int, gate: str, case: str) -> str:
    gate_part = "".join(character for character in gate.lower() if character.isalnum())
    case_part = "".join(
        character for character in case.lower() if character.isalnum() or character in "-_"
    )
    if not gate_part or not case_part:
        fail("next case does not form a safe artifact name")
    return f"{index:03d}-{gate_part}-{case_part}-raw.json"


def substitute_argv(
    argv: list[str], artifact_root: Path | None, artifact_output: Path
) -> tuple[list[str], list[str]]:
    required_inputs: list[str] = []
    output: list[str] = []
    for argument in argv:
        if argument == "<artifact_root>":
            if artifact_root is None:
                output.append(argument)
                required_inputs.append("artifact_root")
            else:
                output.append(str(artifact_root))
        elif argument == "<output_event.json>":
            output.append(str(artifact_output))
        else:
            output.append(argument)
    return output, required_inputs


def next_guidance(
    run_directory: Path,
    artifact_directory: Path | None = None,
    artifact_root: Path | None = None,
) -> dict[str, Any]:
    resolved_run, state, acceptance = journal.load_journal(run_directory)
    plan = read_plan()
    indexed = validate_plan(plan, acceptance)
    next_item = journal.expected_case(acceptance, state["next_index"])
    if next_item is None:
        return {
            "schema": PLAN_SCHEMA,
            "run_directory": str(resolved_run),
            "complete": True,
            "sealed": state.get("sealed") is True,
            "next": None,
            "finalize_argv": [
                "python3",
                "scripts/live_read_evidence_journal.py",
                "finalize",
                str(resolved_run),
            ],
        }
    gate, case = next_item
    entry = indexed[(gate, case)]
    capture_directory = (
        artifact_directory.resolve()
        if artifact_directory is not None
        else (resolved_run / "capture").resolve()
    )
    if capture_directory.is_symlink():
        fail("capture artifact directory must not be a symbolic link")
    artifact_output = capture_directory / safe_artifact_name(
        state["next_index"], gate, case
    )
    resolved_argv, required_inputs = substitute_argv(
        entry["argv"],
        artifact_root.resolve() if artifact_root is not None else None,
        artifact_output,
    )
    ready = not required_inputs and bool(resolved_argv)
    append_argv = [
        "python3",
        "scripts/live_read_evidence_journal.py",
        entry["append_mode"],
        str(resolved_run),
        str(artifact_output),
    ]
    return {
        "schema": PLAN_SCHEMA,
        "run_directory": str(resolved_run),
        "complete": False,
        "sealed": state.get("sealed") is True,
        "recorded_events": len(state["records"]),
        "required_events": len(journal.expected_sequence(acceptance)),
        "next": {
            "gate": gate,
            "case": case,
            "capture_kind": entry["capture_kind"],
            "automatable": entry["automatable"],
            "ready_to_execute": ready,
            "argv": resolved_argv,
            "capture_stdout_to": str(artifact_output)
            if entry["capture_kind"] == "probe"
            else None,
            "event_output": str(artifact_output)
            if entry["capture_kind"] == "scanner"
            else None,
            "append_argv": append_argv,
            "required_inputs": required_inputs,
            "preconditions": entry["preconditions"],
            "operator_action": entry["operator_action"],
        },
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_directory", type=Path)
    parser.add_argument("--artifact-directory", type=Path)
    parser.add_argument("--artifact-root", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        result = next_guidance(
            args.run_directory,
            args.artifact_directory,
            args.artifact_root,
        )
    except (GuidanceError, journal.JournalError, OSError) as exc:
        print(f"live read capture guidance: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
