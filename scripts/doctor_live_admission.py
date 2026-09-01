#!/usr/bin/env python3
"""Diagnose exact live-admission readiness without executing or acquiring authority."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PROMOTION_PATH = ROOT / "scripts/promote_live_compatibility.py"
RESOLVER_PATH = ROOT / "scripts/resolve_live_compatibility.py"
FLOOR_PATH = ROOT / "scripts/live_compatibility_floor.py"
BINARY_VERIFIER_PATH = ROOT / "scripts/verify_live_server_binary_receipt.py"
DEFAULT_REGISTRY = ROOT / "architecture/live_compatibility_registry_v1.json"
REPORT_SCHEMA = "dfmcp.live-admission-doctor/1"
MAX_DIAGNOSTIC_BYTES = 1024
STAGE_ORDER = [
    "registry",
    "compatibility_floor",
    "exact_tuple_resolution",
    "server_artifact",
]
LIMITATIONS = [
    "a passing report does not execute the server",
    "a passing report does not prove a bridge connection",
    "a passing report does not replace R1-R5 compatibility evidence",
    "the report is valid only for the exact input bytes it identifies",
    "the report grants no game or process authority",
]


def load_module(name: str, path: Path) -> Any:
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


promotion = load_module("promote_live_compatibility", PROMOTION_PATH)
resolver = load_module("resolve_live_compatibility", RESOLVER_PATH)
compatibility_floor = load_module("live_compatibility_floor", FLOOR_PATH)
binary_verifier = load_module("verify_live_server_binary_receipt", BINARY_VERIFIER_PATH)


def bounded_text(value: object) -> str:
    text = str(value)
    normalized = "".join(
        character if ord(character) >= 0x20 or character in "\t\n\r" else "?"
        for character in text
    )
    encoded = normalized.encode("utf-8")
    if len(encoded) <= MAX_DIAGNOSTIC_BYTES:
        return normalized
    return encoded[:MAX_DIAGNOSTIC_BYTES].decode("utf-8", errors="ignore")


def recovery(action: str, reason: str, arguments: dict[str, Any]) -> dict[str, Any]:
    return {
        "action": action,
        "reason": reason,
        "arguments": arguments,
    }


def stage(
    name: str,
    status: str,
    summary: object,
    evidence: dict[str, Any] | None = None,
    next_step: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "stage": name,
        "status": status,
        "summary": bounded_text(summary),
        "evidence": {} if evidence is None else evidence,
        "recovery": next_step,
    }


def not_checked(name: str, dependency: str) -> dict[str, Any]:
    return stage(
        name,
        "not_checked",
        f"not checked because {dependency} did not pass",
    )


def finish(
    status: str,
    stages: list[dict[str, Any]],
    registry: dict[str, Any] | None,
    floor_value: dict[str, Any] | None,
    decision: dict[str, Any] | None,
    artifact: dict[str, Any] | None,
) -> dict[str, Any]:
    if [item["stage"] for item in stages] != STAGE_ORDER:
        raise ValueError("live admission doctor stage order drifted")
    unsigned: dict[str, Any] = {
        "schema": REPORT_SCHEMA,
        "status": status,
        "stages": stages,
        "registry": registry,
        "compatibility_floor": floor_value,
        "compatibility_decision": decision,
        "server_artifact": artifact,
        "authority": {
            "executes_server": False,
            "connects_to_dfhack": False,
            "reads_bridge_token": False,
            "modifies_registry": False,
            "modifies_floor": False,
            "grants_capabilities": [],
            "mutation_capabilities": [],
        },
        "limitations": LIMITATIONS,
    }
    return {
        **unsigned,
        "report_digest": promotion.sha256_bytes(promotion.canonical_json(unsigned)),
    }


def artifact_inputs(
    binary_path: Path | None,
    server_receipt_path: Path | None,
    local_qualification_receipt: Path | None,
    binary_contract_path: Path | None,
    source_root: Path | None,
    expected_dfmcp_commit: str | None,
) -> tuple[bool, tuple[Path, Path, Path, Path, Path, str] | None]:
    values: tuple[object | None, ...] = (
        binary_path,
        server_receipt_path,
        local_qualification_receipt,
        binary_contract_path,
        source_root,
        expected_dfmcp_commit,
    )
    supplied = [value is not None for value in values]
    if any(supplied) and not all(supplied):
        raise ValueError("server artifact inputs are all-or-none")
    if not any(supplied):
        return False, None
    if (
        binary_path is None
        or server_receipt_path is None
        or local_qualification_receipt is None
        or binary_contract_path is None
        or source_root is None
        or expected_dfmcp_commit is None
    ):
        raise ValueError("complete server artifact inputs were not retained")
    return True, (
        binary_path,
        server_receipt_path,
        local_qualification_receipt,
        binary_contract_path,
        source_root,
        expected_dfmcp_commit,
    )


def diagnose(
    manifest_path: Path,
    registry_path: Path,
    floor_path: Path,
    required_entry_id: str,
    *,
    binary_path: Path | None = None,
    server_receipt_path: Path | None = None,
    local_qualification_receipt: Path | None = None,
    binary_contract_path: Path | None = None,
    source_root: Path | None = None,
    expected_dfmcp_commit: str | None = None,
) -> dict[str, Any]:
    has_artifact, artifact_parameters = artifact_inputs(
        binary_path,
        server_receipt_path,
        local_qualification_receipt,
        binary_contract_path,
        source_root,
        expected_dfmcp_commit,
    )
    stages: list[dict[str, Any]] = []
    registry_summary: dict[str, Any] | None = None
    floor_summary: dict[str, Any] | None = None
    decision_summary: dict[str, Any] | None = None
    artifact_summary: dict[str, Any] | None = None

    try:
        registry_value, registry_file_sha256 = promotion.read_object_with_digest(
            registry_path, promotion.MAX_JSON_BYTES, "compatibility registry"
        )
        entries = promotion.validate_registry(registry_value)
        generation = compatibility_floor.registry_generation_from_value(
            registry_value, registry_file_sha256
        )
        registry_summary = {
            "file_sha256": registry_file_sha256,
            "canonical_digest": generation["registry_digest"],
            "status": registry_value["status"],
            "entry_count": len(entries),
            "entry_ids": list(generation["entry_ids"]),
        }
        stages.append(
            stage(
                "registry",
                "passed",
                "compatibility registry is structurally valid and canonically identified",
                registry_summary,
            )
        )
    except (OSError, promotion.PromotionError) as exc:
        stages.append(
            stage(
                "registry",
                "failed",
                exc,
                next_step=recovery(
                    "repair_registry",
                    "replace the registry only with a reviewed generation that passes the exact schema",
                    {"registry": os.fspath(registry_path)},
                ),
            )
        )
        stages.extend(
            [
                not_checked("compatibility_floor", "registry validation"),
                not_checked("exact_tuple_resolution", "registry validation"),
                not_checked("server_artifact", "registry validation"),
            ]
        )
        return finish(
            "not_ready",
            stages,
            registry_summary,
            floor_summary,
            decision_summary,
            artifact_summary,
        )

    try:
        floor_value, floor_file_sha256 = compatibility_floor.read_floor(floor_path)
        compatibility_floor.verify_generation(floor_value, generation)
        floor_summary = {
            "file_sha256": floor_file_sha256,
            "floor_digest": floor_value["floor_digest"],
            "sequence": floor_value["sequence"],
            "registry_file_sha256": floor_value["registry_file_sha256"],
            "registry_digest": floor_value["registry_digest"],
            "entry_count": len(floor_value["entry_ids"]),
        }
        stages.append(
            stage(
                "compatibility_floor",
                "passed",
                "owner-private monotonic floor exactly matches the selected registry generation",
                floor_summary,
            )
        )
    except (
        OSError,
        compatibility_floor.FloorError,
        compatibility_floor.promotion.PromotionError,
    ) as exc:
        stages.append(
            stage(
                "compatibility_floor",
                "failed",
                exc,
                next_step=recovery(
                    "verify_or_advance_floor",
                    "inspect floor custody and advance it only through compare-and-swap after reviewing the registry generation",
                    {
                        "floor": os.fspath(floor_path),
                        "registry": os.fspath(registry_path),
                    },
                ),
            )
        )
        stages.extend(
            [
                not_checked("exact_tuple_resolution", "compatibility floor verification"),
                not_checked("server_artifact", "compatibility floor verification"),
            ]
        )
        return finish(
            "not_ready",
            stages,
            registry_summary,
            floor_summary,
            decision_summary,
            artifact_summary,
        )

    try:
        normalized_required_entry_id = promotion.require_hash(
            required_entry_id, "required_entry_id"
        )
        manifest_value = promotion.read_object(
            manifest_path, 1024 * 1024, "deployment manifest"
        )
        decision = resolver.resolve(
            registry_value, manifest_value, normalized_required_entry_id
        )
        if decision["admitted"] is not True:
            reasons = "; ".join(decision["reasons"]) or "exact tuple was not admitted"
            raise ValueError(reasons)
        if decision["registry_digest"] != floor_value["registry_digest"]:
            raise ValueError("compatibility decision and monotonic floor name different registries")
        decision_summary = {
            "decision_digest": decision["decision_digest"],
            "entry_id": decision["entry_id"],
            "required_entry_id": decision["required_entry_id"],
            "support_level": decision["support_level"],
            "registry_digest": decision["registry_digest"],
            "manifest": decision["manifest"],
            "capabilities": list(decision["capabilities"]),
            "mutation_capabilities": [],
            "omitted_domains": list(decision["omitted_domains"]),
        }
        stages.append(
            stage(
                "exact_tuple_resolution",
                "passed",
                "deployment manifest resolves to the explicitly required exact entry",
                decision_summary,
            )
        )
    except (
        OSError,
        ValueError,
        promotion.PromotionError,
        resolver.ResolutionError,
        resolver.promotion.PromotionError,
    ) as exc:
        stages.append(
            stage(
                "exact_tuple_resolution",
                "failed",
                exc,
                next_step=recovery(
                    "qualify_exact_tuple",
                    "capture R1-R5 evidence for the exact source, binary, version, and platform tuple or select the correct admitted manifest",
                    {
                        "manifest": os.fspath(manifest_path),
                        "required_entry_id": required_entry_id,
                    },
                ),
            )
        )
        stages.append(not_checked("server_artifact", "exact tuple resolution"))
        return finish(
            "not_ready",
            stages,
            registry_summary,
            floor_summary,
            decision_summary,
            artifact_summary,
        )

    if not has_artifact:
        stages.append(
            stage(
                "server_artifact",
                "not_checked",
                "no server artifact inputs were supplied; compatibility readiness only",
                next_step=recovery(
                    "qualify_server_artifact",
                    "run local qualification and produce a source-bound release-server receipt before launch",
                    {},
                ),
            )
        )
        return finish(
            "compatibility_ready",
            stages,
            registry_summary,
            floor_summary,
            decision_summary,
            artifact_summary,
        )

    if artifact_parameters is None or decision_summary is None:
        raise ValueError("artifact preflight reached an impossible incomplete state")
    opened = None
    try:
        (
            artifact_binary,
            artifact_receipt,
            artifact_local_receipt,
            artifact_contract,
            artifact_source_root,
            artifact_expected_commit,
        ) = artifact_parameters
        if decision_summary["manifest"]["source"]["dfmcp_commit"] != artifact_expected_commit:
            raise ValueError(
                "artifact expected commit differs from the admitted deployment manifest"
            )
        normalized_receipt, opened = binary_verifier.verify(
            artifact_receipt,
            artifact_binary,
            artifact_contract,
            artifact_source_root,
            artifact_local_receipt,
            artifact_expected_commit,
        )
        if (
            normalized_receipt["source"]["dfmcp_commit"]
            != decision_summary["manifest"]["source"]["dfmcp_commit"]
        ):
            raise ValueError(
                "server receipt and compatibility decision name different source commits"
            )
        if normalized_receipt["platform"] != decision_summary["manifest"]["platform"]:
            raise ValueError(
                "server receipt and compatibility decision name different platforms"
            )
        artifact_summary = {
            "receipt_file_sha256": normalized_receipt["receipt_sha256"],
            "receipt_digest": normalized_receipt["receipt_digest"],
            "local_qualification_receipt_sha256": normalized_receipt["source"][
                "local_qualification_receipt_sha256"
            ],
            "dfmcp_commit": normalized_receipt["source"]["dfmcp_commit"],
            "platform": normalized_receipt["platform"],
            "binary": {
                "path": os.fspath(opened.path),
                "sha256": opened.sha256,
                "bytes": opened.size,
                "device": opened.device,
                "inode": opened.inode,
                "mode": opened.mode,
                "owner_uid": opened.owner_uid,
            },
            "mutation_capabilities": [],
        }
        stages.append(
            stage(
                "server_artifact",
                "passed",
                "source-bound server receipt and already-opened binary identity passed preflight",
                artifact_summary,
            )
        )
        return finish(
            "artifact_preflight_ready",
            stages,
            registry_summary,
            floor_summary,
            decision_summary,
            artifact_summary,
        )
    except (
        OSError,
        KeyError,
        TypeError,
        ValueError,
        binary_verifier.VerificationError,
    ) as exc:
        stages.append(
            stage(
                "server_artifact",
                "failed",
                exc,
                next_step=recovery(
                    "requalify_server_artifact",
                    "re-run local qualification and server binary qualification for the exact admitted source revision",
                    {},
                ),
            )
        )
        return finish(
            "not_ready",
            stages,
            registry_summary,
            floor_summary,
            decision_summary,
            artifact_summary,
        )
    finally:
        if opened is not None:
            try:
                os.close(opened.descriptor)
            except OSError:
                pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--compatibility-floor", type=Path, required=True)
    parser.add_argument("--require-entry-id", required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--server-receipt", type=Path)
    parser.add_argument("--local-qualification-receipt", type=Path)
    parser.add_argument("--binary-contract", type=Path)
    parser.add_argument("--source-root", type=Path)
    parser.add_argument("--expected-dfmcp-commit")
    parser.add_argument("--output", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        report = diagnose(
            args.manifest,
            args.registry,
            args.compatibility_floor,
            args.require_entry_id,
            binary_path=args.binary,
            server_receipt_path=args.server_receipt,
            local_qualification_receipt=args.local_qualification_receipt,
            binary_contract_path=args.binary_contract,
            source_root=args.source_root,
            expected_dfmcp_commit=args.expected_dfmcp_commit,
        )
    except ValueError as exc:
        print(f"live admission doctor: FAIL: {bounded_text(exc)}", file=sys.stderr)
        return 2
    if args.output is None:
        print(json.dumps(report, sort_keys=True, separators=(",", ":"), ensure_ascii=False))
    else:
        promotion.write_atomic(args.output, report)
    return 0 if report["status"] != "not_ready" else 3


if __name__ == "__main__":
    raise SystemExit(main())
