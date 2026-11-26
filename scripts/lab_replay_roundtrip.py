#!/usr/bin/env python3
"""Record a laboratory session over real stdio, export its replay bundle from
df://session/{id}/replay, and re-execute it with `dwarf-fortress-mcp replay`.
Then change a read result without changing the world, and separately change a
wait, showing the replayer localizing both kinds of divergence.

Usage: python3 scripts/lab_replay_roundtrip.py [path/to/dwarf-fortress-mcp] [--output bundle.json]
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", nargs="?", default="target/debug/dwarf-fortress-mcp")
parser.add_argument("--output", type=Path, help="save the original verified UTF-8 replay bundle")
args = parser.parse_args()
BINARY = args.binary
META = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {"tools": {"listChanged": True}},
        "io.modelcontextprotocol/clientInfo": {"name": "replay", "version": "0.0.1"}}
p = subprocess.Popen([BINARY, "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, text=True)
n = [0]
def call(method, params):
    n[0] += 1
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n[0], "method": method, "params": dict(params, _meta=META)}) + "\n")
    p.stdin.flush()
    while True:
        line = p.stdout.readline()
        if not line:
            raise SystemExit("eof")
        if line.lstrip().startswith("{"):
            msg = json.loads(line)
            if msg.get("id") == n[0]:
                return msg
def tool(name, args):
    return json.loads(call("tools/call", {"name": name, "arguments": args})["result"]["content"][0]["text"])
call("server/discover", {})
caps = [["observe", "read_only"], ["query", "read_only"], ["plan", "reversible"], ["control_clock", "reversible"], ["designate", "guarded"]]
sid = tool("fortress_open_session", {"paused": False, "scenario": "starter_fortress", "requested_capabilities": caps, "max_game_ticks": 5000})["session_id"]
tool("fortress_query", {"session_id": sid,
                         "mode": json.dumps({"mode": "entities", "kind": "unit", "limit": 5})})
pl = tool("fortress_plan", {"session_id": sid, "actions": json.dumps([{"action": {"kind": "designate_dig", "min": [1, 3, 10], "max": [5, 5, 10], "mode": "mine"}}])})
tool("fortress_commit", {"session_id": sid, "plan_digest": pl["plan_digest"]})
for _ in range(3):
    tool("fortress_wait", {"session_id": sid, "max_game_ticks": 60})
def read_resource(uri):
    return call("resources/read", {"uri": uri})["result"]["contents"][0]["text"]

def export_document(uri):
    text = read_resource(uri)
    page = json.loads(text)
    if page.get("schema") != "dfmcp.replay.page/1":
        return text.encode("utf-8")
    digest, total = page["digest"], page["total_bytes"]
    document = bytearray()
    while True:
        if (page["schema"] != "dfmcp.replay.page/1" or page["digest"] != digest
                or page["total_bytes"] != total or page["offset"] != len(document)):
            raise ValueError("replay page does not continue the same document")
        part = page["part"].encode("utf-8")
        if not part or len(document) + len(part) > total:
            raise ValueError("invalid replay page length")
        document.extend(part)
        if page["complete"]:
            if page["next"] is not None:
                raise ValueError("completed replay page has a continuation")
            break
        page = json.loads(read_resource(page["next"]))
    if len(document) != total or hashlib.sha256(document).hexdigest() != digest:
        raise ValueError("reassembled replay bundle differs from its digest or byte count")
    return bytes(document)

try:
    document = export_document(f"df://session/{sid}/replay")
finally:
    p.stdin.close()
    p.wait(timeout=10)

bundle = json.loads(document)
if bundle.get("schema") != "dfmcp.replay.bundle/3":
    raise ValueError("this example requires a server recording semantic replay bundle v3")
print("recorded calls:", len(bundle["calls"]), "replayable:", bundle["replayable"])

def reseal(value):
    calls = json.dumps(value["calls"], separators=(",", ":"), sort_keys=True, ensure_ascii=False)
    value["calls_digest"] = hashlib.sha256(b"dfmcp-replay-bundle-calls/3\0" + calls.encode()).hexdigest()

with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "bundle.json"
    path.write_bytes(document)
    out = subprocess.run([BINARY, "replay", path], capture_output=True, text=True, check=True)
    report = json.loads(out.stdout)
    if report["ok"] is not True or report["replayed"] != len(bundle["calls"]):
        raise ValueError(f"replay did not reproduce every recorded call: {report}")
    verification = report.get("verification") or {}
    if (verification.get("mode") != "semantic_tool_results"
            or verification.get("semantic_result_calls", 0) < 1):
        raise ValueError(f"replay did not verify semantic tool results: {report}")
    print("replay:", out.stdout.strip())

    for target, field in [("fortress.query", "result_commitment"), ("fortress.wait", "anchor_after")]:
        changed = copy.deepcopy(bundle)
        seq = next(i for i, entry in enumerate(changed["calls"]) if entry.get("tool") == target)
        entry = changed["calls"][seq]
        if target == "fortress.query":
            if entry["result_commitment"]["profile"] != "dfmcp.replay.tool-payload/1":
                raise ValueError("entities query was excluded from semantic replay verification")
            query = json.loads(entry["arguments"]["mode"])
            query["limit"] = 1
            entry["arguments"]["mode"] = json.dumps(query)
        else:
            entry["arguments"]["max_game_ticks"] = 59
        # Recompute transport integrity so the verifier must compare execution,
        # rather than merely reject an out-of-date bundle checksum.
        reseal(changed)
        path.write_text(json.dumps(changed), encoding="utf-8")
        out = subprocess.run([BINARY, "replay", path], capture_output=True, text=True)
        report = json.loads(out.stdout)
        divergence = report.get("first_divergence") or {}
        if (out.returncode == 0 or report.get("ok") is not False or divergence.get("seq") != seq
                or divergence.get("tool") != target or divergence.get("field") != field):
            raise ValueError(f"replayer failed to localize altered {target}: {report}")
        print("tampered replay:", out.stdout.strip())

if args.output:
    with tempfile.TemporaryDirectory(dir=args.output.parent) as directory:
        staged = Path(directory) / "bundle.json"
        staged.write_bytes(document)
        staged.replace(args.output)
    print("saved verified replay:", args.output)
print("ok")
