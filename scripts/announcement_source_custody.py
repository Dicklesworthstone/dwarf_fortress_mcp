"""Full, private source custody for unadmitted announcement qualification."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import write_local_qualification_receipt as custody


def capture(root: Path, contract: Path, snapshot: Path, commit: str) -> dict[str, Any]:
    return custody.begin(root, contract, snapshot, commit, False)


def verify(root: Path, contract: Path, snapshot: Path, commit: str) -> dict[str, Any]:
    policy = custody.load_contract(contract)
    raw = custody.read_private_evidence_file(
        snapshot, custody.MAX_JSON_BYTES, "announcement qualification source snapshot"
    )
    expected = custody.validate_snapshot(
        custody.read_json_bytes(raw.content, "announcement qualification source snapshot")
    )
    if expected["source"]["commit"] != commit:
        custody.fail("announcement qualification snapshot commit differs")
    if expected["source"]["dirty"] or not expected["source"]["head_equivalent"]:
        custody.fail("announcement qualification requires clean HEAD-equivalent source")
    if custody.collect_source_snapshot(root, policy, commit) != expected:
        custody.fail("source changed during announcement source qualification")
    return expected


def publish(
    root: Path,
    contract: Path,
    snapshot: Path,
    commit: str,
    output: Path,
    unsigned: dict[str, Any],
) -> dict[str, Any]:
    destination, parent = custody.private_evidence_candidate(output)
    custody.private_evidence_candidate(snapshot, parent)
    source = verify(root, contract, snapshot, commit)
    value = dict(unsigned)
    value["source"] = {
        **source["source"],
        "snapshot_digest": source["snapshot_digest"],
        "entries_digest": source["entries_digest"],
        "entry_count": len(source["entries"]),
    }
    if "receipt_digest" in value:
        custody.fail("announcement receipt input must be unsigned")
    receipt = {
        **value,
        "receipt_digest": custody.sha256_bytes(custody.canonical_json(value)),
    }
    # Recheck after preparing the payload and after publishing its exact bytes.
    if verify(root, contract, snapshot, commit) != source:
        custody.fail("source changed while preparing announcement receipt")
    published = custody.write_atomic_create_only(destination, receipt)
    try:
        if verify(root, contract, snapshot, commit) != source:
            custody.fail("source changed while publishing announcement receipt")
    except BaseException:
        custody.remove_published_evidence(published, "announcement source qualification receipt")
        raise
    return receipt
