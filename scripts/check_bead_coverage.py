#!/usr/bin/env python3
"""Bead-graph coverage governance (bead df-bead-graph-governance-pq2).

Enforces mechanically:
  1. Every WP-xx id defined in design/registries/WORK_PACKAGES.md maps to at least one
     bead (open or closed) whose id or title carries that WP marker.
  2. Every `#[ignore]` reason in crates/ references at least one known bead id.

Runs as part of scripts/verify.sh. Exit 1 on any violation.
"""
import json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WP_FILE = os.path.join(ROOT, "design", "registries", "WORK_PACKAGES.md")

def bead_ids():
    out = subprocess.run(["br", "list", "--status=all", "--json"], capture_output=True, text=True)
    if out.returncode != 0:
        print(f"FAIL: br list failed: {out.stderr.strip()[:200]}")
        sys.exit(1)
    issues = json.loads(out.stdout).get("issues", [])
    return [i["id"] for i in issues], [i.get("title", "") for i in issues]

def wp_ids():
    text = open(WP_FILE, encoding="utf-8").read()
    return sorted(set(re.findall(r"\bWP-\d{2}\b", text)))

def wp_marker_matches(wp, bid, title):
    n = wp.split("-")[1].lower()               # e.g. "07"
    needle = f"wp{n}"                           # wp07
    blob = f"{bid.lower()} {title.lower()}"
    return needle in blob or wp.lower() in blob

def main():
    ids, titles = bead_ids()
    failures = []

    known = "\n".join(ids)
    for wp in wp_ids():
        hits = [bid for bid, title in zip(ids, titles) if wp_marker_matches(wp, bid, title)]
        if not hits:
            failures.append(f"WP-coverage: {wp} has no bead (id/title lacks '{wp.split('-')[1].lower()}')")
    print(f"bead coverage: {len(wp_ids())} WPs checked, "
          f"{len(wp_ids()) - sum(1 for f in failures if f.startswith('WP-coverage'))} covered")

    ignore_re = re.compile(r'(?m)^\s*#\[(?:"[^"]*"|[^"\]])*?ignore\s*=\s*"((?:[^"\\]|\\.)*)"\s*\]', re.S)
    bare_ignore_re = re.compile(r"(?m)^\s*#\[ignore\]\s*$")
    for dirpath, _dirs, files in os.walk(os.path.join(ROOT, "crates")):
        for fn in files:
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dirpath, fn)
            text = open(path, encoding="utf-8").read()
            for m in ignore_re.finditer(text):
                reason = m.group(1) or ""
                if not any(bid in reason for bid in ids):
                    rel = os.path.relpath(path, ROOT)
                    failures.append(
                        f"ignore-ref: {rel}: #[ignore] reason cites no known bead id "
                        f"(reason starts: {reason[:80]!r})")
            if bare_ignore_re.search(text):
                rel = os.path.relpath(path, ROOT)
                failures.append(
                    f"ignore-ref: {rel}: bare #[ignore] carries no reason; "
                    "cite a bead id explaining the block")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        sys.exit(1)
    print("bead coverage: OK")

if __name__ == "__main__":
    main()
