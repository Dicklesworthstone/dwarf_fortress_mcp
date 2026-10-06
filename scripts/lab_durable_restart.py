#!/usr/bin/env python3
"""Prove a crash-durable laboratory fortress survives a real process restart:
start `dwarf-fortress-mcp serve` with DFMCP_LAB_STATE_DIR, designate an
excavation and checkpoint it, kill the server with SIGKILL mid-work, start a
new server on the same directory, and resume the fortress. Laboratory
semantics only.

Usage: python3 scripts/lab_durable_restart.py [path/to/dwarf-fortress-mcp]
"""
import json, os, subprocess, sys, tempfile

BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/debug/dwarf-fortress-mcp"
META = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {"tools": {"listChanged": True}},
        "io.modelcontextprotocol/clientInfo": {"name": "durable-restart", "version": "0.0.1"}}
CAPS = [["observe", "read_only"], ["query", "read_only"], ["plan", "reversible"],
        ["control_clock", "reversible"], ["checkpoint", "guarded"], ["restore", "guarded"],
        ["doctor", "read_only"], ["designate", "guarded"]]
TERRAIN = json.dumps({"mode": "terrain", "min": [0, 3, 10], "max": [7, 5, 10]})


class Server:
    def __init__(self, state_dir):
        env = dict(os.environ, DFMCP_LAB_STATE_DIR=state_dir)
        self.p = subprocess.Popen([BINARY, "serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, env=env)
        self.n = 0
        self.call("server/discover", {})

    def call(self, method, params):
        self.n += 1
        params = dict(params, _meta=META)
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise SystemExit("server closed stdout")
            if line.lstrip().startswith("{"):
                msg = json.loads(line)
                if msg.get("id") == self.n:
                    return msg

    def tool(self, name, args):
        return json.loads(self.call("tools/call", {"name": name, "arguments": args})["result"]["content"][0]["text"])

    def kill(self):
        self.p.kill()
        self.p.wait(timeout=10)


def open_durable(server, scenario=None):
    args = {"paused": False, "fortress_selector": "4711", "durable": True,
            "requested_capabilities": CAPS, "max_game_ticks": 5000}
    if scenario:
        args["scenario"] = scenario
    opened = server.tool("fortress_open_session", args)
    assert opened["ok"], opened
    return opened


with tempfile.TemporaryDirectory() as root:
    state = os.path.join(root, "lab-state")
    first = Server(state)
    opened = open_durable(first, "starter_fortress")
    sid = opened["session_id"]
    print("first process: resumed =", opened["durable"]["resumed"])
    actions = [{"action": {"kind": "designate_dig", "min": [0, 3, 10], "max": [7, 5, 10], "mode": "mine"}}]
    plan = first.tool("fortress_plan", {"session_id": sid, "actions": json.dumps(actions)})
    commit = first.tool("fortress_commit", {"session_id": sid, "plan_digest": plan["plan_digest"]})
    assert commit["ok"], commit
    first.tool("fortress_wait", {"session_id": sid, "max_game_ticks": 60})
    cp = first.tool("fortress_checkpoint", {"session_id": sid, "label": "six tiles dug"})
    assert cp["durable"], cp
    first.tool("fortress_wait", {"session_id": sid, "max_game_ticks": 60})
    before = first.tool("fortress_query", {"session_id": sid, "mode": TERRAIN})["levels"][0]["rows"]
    print("before SIGKILL:", before)
    first.kill()

    second = Server(state)
    resumed = open_durable(second)
    sid2 = resumed["session_id"]
    print("second process: resumed =", resumed["durable"]["resumed"],
          "checkpoints =", resumed["durable"]["restorable_checkpoints"],
          "epoch", resumed["durable"]["recovered_from_anchor"]["epoch"], "->", resumed["anchor"]["epoch"])
    after = second.tool("fortress_query", {"session_id": sid2, "mode": TERRAIN})["levels"][0]["rows"]
    assert after == before, (before, after)
    for _ in range(10):
        second.tool("fortress_wait", {"session_id": sid2, "max_game_ticks": 50})
    done = second.tool("fortress_query", {"session_id": sid2, "mode": TERRAIN})["levels"][0]["rows"]
    print("work continued after restart:", done)
    assert done == ["........"] * 3, done
    restored = second.tool("fortress_restore", {"session_id": sid2, "checkpoint_id": cp["checkpoint_id"]})
    assert restored["ok"], restored
    print("restored pre-crash checkpoint:",
          second.tool("fortress_query", {"session_id": sid2, "mode": TERRAIN})["levels"][0]["rows"])
    doctor = second.tool("fortress_doctor", {"session_id": sid2})
    print("doctor durability:", json.dumps(doctor["durability"]["store"]))
    second.kill()
print("ok")
