#!/usr/bin/env python3
"""Static authority and epistemic-law checks for dfmcp-core.

Cargo tests remain the executable reference. This checker provides earlier,
static-only evidence that the source has not regressed in several places where
a type or method can look plausible while carrying the wrong authority meaning.
"""

from __future__ import annotations

import pathlib
import re
import sys
from dataclasses import dataclass

ROOT = pathlib.Path(__file__).resolve().parents[1]
AGENT = ROOT / "crates/dfmcp-core/src/agent.rs"
MODEL = ROOT / "crates/dfmcp-core/src/model.rs"
CAPABILITY_TEST = ROOT / "crates/dfmcp-core/tests/capability_use_contract.rs"


@dataclass(frozen=True)
class Failure:
    path: str
    message: str


def read(path: pathlib.Path, failures: list[Failure]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        failures.append(Failure(str(path.relative_to(ROOT)), f"cannot read file: {exc}"))
        return ""


def block(source: str, marker: str) -> str | None:
    start = source.find(marker)
    if start < 0:
        return None
    opening = source.find("{", start)
    if opening < 0:
        return None
    depth = 0
    for offset in range(opening, len(source)):
        char = source[offset]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[opening + 1 : offset]
    return None


def production_rust_sources() -> list[pathlib.Path]:
    output: list[pathlib.Path] = []
    for path in sorted((ROOT / "crates").rglob("*.rs")):
        relative = path.relative_to(ROOT)
        if "tests" in relative.parts or "target" in relative.parts:
            continue
        output.append(path)
    return output


def main() -> int:
    failures: list[Failure] = []
    agent = read(AGENT, failures)
    model = read(MODEL, failures)
    capability_test = read(CAPABILITY_TEST, failures)

    affordance = block(agent, "pub struct Affordance")
    if affordance is None:
        failures.append(Failure(str(AGENT.relative_to(ROOT)), "Affordance struct is missing"))
    else:
        if "pub id: AffordanceId" not in affordance:
            failures.append(
                Failure(
                    str(AGENT.relative_to(ROOT)),
                    "Affordance does not carry its dedicated AffordanceId",
                )
            )
        if "pub id: RecommendationId" in affordance:
            failures.append(
                Failure(
                    str(AGENT.relative_to(ROOT)),
                    "Affordance incorrectly aliases recommendation identity",
                )
            )

    absence = block(agent, "pub fn proves_absence_in")
    if absence is None or "self.anchor.is_some()" not in absence:
        failures.append(
            Failure(
                str(AGENT.relative_to(ROOT)),
                "absence proof is not explicitly conditioned on an anchor",
            )
        )

    surprise = block(agent, "impl SurpriseRecord")
    if surprise is None:
        failures.append(
            Failure(str(AGENT.relative_to(ROOT)), "SurpriseRecord implementation is missing")
        )
    elif "predicted_digest == self.observed_digest" in surprise:
        failures.append(
            Failure(
                str(AGENT.relative_to(ROOT)),
                "surprise validation incorrectly assumes equal world digests imply no surprise",
            )
        )
    if "timing_surprise_can_preserve_the_world_state_digest" not in agent:
        failures.append(
            Failure(
                str(AGENT.relative_to(ROOT)),
                "timing-surprise equal-digest regression test is missing",
            )
        )

    continuity = block(agent, "impl Continuity")
    for marker in [
        "gap continuity does not describe a valid missing interval",
        "bootstrap continuity requires only a target anchor",
        "reset continuity requires a later same-fortress epoch",
    ]:
        if continuity is None or marker not in continuity:
            failures.append(
                Failure(
                    str(AGENT.relative_to(ROOT)),
                    f"continuity validation is missing {marker!r}",
                )
            )

    for marker in [
        "pub fn authorize_and_consume",
        "limited-use grant; call authorize_and_consume",
        "fn consume_one_use",
        "immutable_authorization_rejects_limited_grants",
        "consuming_authorization_exhausts_a_one_use_grant",
    ]:
        if marker not in model:
            failures.append(
                Failure(
                    str(MODEL.relative_to(ROOT)),
                    f"limited capability-use contract is missing {marker!r}",
                )
            )

    if "limited_grants_are_not_recreated_in_production_request_paths" not in capability_test:
        failures.append(
            Failure(
                str(CAPABILITY_TEST.relative_to(ROOT)),
                "persistent-consumption guard test is missing",
            )
        )

    production_violations = []
    for path in production_rust_sources():
        if path == MODEL:
            continue
        source = read(path, failures)
        prefix = source.split("#[cfg(test)]", 1)[0]
        if "remaining_uses: Some(" in prefix:
            production_violations.append(str(path.relative_to(ROOT)))
    if production_violations:
        failures.append(
            Failure(
                "crates",
                "limited-use grants are reconstructed without an authoritative persistent ledger in "
                + ", ".join(production_violations),
            )
        )

    if re.search(r"pub\s+id:\s*RecommendationId", affordance or ""):
        failures.append(
            Failure(
                str(AGENT.relative_to(ROOT)),
                "affordance identity regression detected by structural pattern",
            )
        )

    if failures:
        print(
            f"core authority contract: FAIL ({len(failures)} violation(s))",
            file=sys.stderr,
        )
        for failure in failures:
            print(f"  {failure.path}: {failure.message}", file=sys.stderr)
        return 1

    print("core authority contract: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
