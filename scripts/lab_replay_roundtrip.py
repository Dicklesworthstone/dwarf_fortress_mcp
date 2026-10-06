#!/usr/bin/env python3
"""Record a laboratory session over real stdio, export its replay bundle from
df://session/{id}/replay, and re-execute it with `dwarf-fortress-mcp replay`.
Then tamper with one recorded call and show the replayer localizing it.

Usage: python3 scripts/lab_replay_roundtrip.py [path/to/dwarf-fortress-mcp]
"""
import json, subprocess, sys, tempfile, os

BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/debug/dwarf-fortress-mcp"
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
pl = tool("fortress_plan", {"session_id": sid, "actions": json.dumps([{"action": {"kind": "designate_dig", "min": [1, 3, 10], "max": [5, 5, 10], "mode": "mine"}}])})
tool("fortress_commit", {"session_id": sid, "plan_digest": pl["plan_digest"]})
for _ in range(3):
    tool("fortress_wait", {"session_id": sid, "max_game_ticks": 60})
res = call("resources/read", {"uri": f"df://session/{sid}/replay"})
bundle = json.loads(res["result"]["contents"][0]["text"])
p.stdin.close(); p.wait(timeout=10)
print("recorded calls:", len(bundle["calls"]), "replayable:", bundle["replayable"])
with tempfile.TemporaryDirectory() as d:
    path = os.path.join(d, "bundle.json")
    json.dump(bundle, open(path, "w"))
    out = subprocess.run([BINARY, "replay", path], capture_output=True, text=True)
    print("replay:", out.stdout.strip(), "exit", out.returncode)
    assert out.returncode == 0
    bundle["calls"][3]["arguments"]["max_game_ticks"] = 59
    import hashlib
    calls = json.dumps(bundle["calls"], separators=(",", ":"), sort_keys=True)
    bundle["calls_digest"] = hashlib.sha256(b"dfmcp-replay-bundle-calls/1\0" + calls.encode()).hexdigest()
    json.dump(bundle, open(path, "w"))
    out = subprocess.run([BINARY, "replay", path], capture_output=True, text=True)
    print("tampered replay:", out.stdout.strip(), "exit", out.returncode)
    assert out.returncode != 0
print("ok")
