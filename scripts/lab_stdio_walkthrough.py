#!/usr/bin/env python3
"""Drive the deterministic MCP laboratory over real stdio: open the
starter_fortress scenario, dig a room, build a still in it, brew, and let game
time pass until every obligation is proven. Laboratory semantics only.

Usage: python3 scripts/lab_stdio_walkthrough.py [path/to/dwarf-fortress-mcp]
"""
import json, subprocess, sys
meta={"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{"tools":{"listChanged":True}},"io.modelcontextprotocol/clientInfo":{"name":"drive","version":"0.0.1"}}
BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/debug/dwarf-fortress-mcp"
p=subprocess.Popen([BINARY,"serve"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True)
n=[0]
def call(method, params):
    n[0]+=1
    params=dict(params); params["_meta"]=meta
    p.stdin.write(json.dumps({"jsonrpc":"2.0","id":n[0],"method":method,"params":params})+"\n"); p.stdin.flush()
    while True:
        line=p.stdout.readline()
        if not line: raise SystemExit("eof")
        if not line.lstrip().startswith("{"): continue
        msg=json.loads(line)
        if msg.get("id")==n[0]: return msg
def tool(name, args):
    r=call("tools/call",{"name":name,"arguments":args})
    return json.loads(r["result"]["content"][0]["text"])
call("server/discover",{})
tools=call("tools/list",{})["result"]["tools"]
schema={t["name"]:sorted(t["inputSchema"]["properties"].keys()) for t in tools}
print("plan params:",schema["fortress_plan"]); print("wait params:",schema["fortress_wait"]); print("open has scenario:","scenario" in schema["fortress_open_session"])
caps=[["observe","read_only"],["query","read_only"],["plan","reversible"],["control_clock","reversible"],["checkpoint","guarded"],["designate","guarded"],["construct","guarded"],["configure_production","reversible"]]
o=tool("fortress_open_session",{"paused":False,"scenario":"starter_fortress","requested_capabilities":caps,"max_game_ticks":2000})
sid=o["session_id"]; print("open ok",o["ok"],o["granted_capabilities"])
actions=[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[4,5,10],"mode":"mine"}},{"action":{"kind":"build","building":"workshop:Still","location":[2,4,10],"min":[1,3,10],"max":[3,5,10]},"depends_on":[0]},{"action":{"kind":"create_work_order","name":"brew","job_token":"BREW_DRINK","amount":2},"depends_on":[1]}]
pl=tool("fortress_plan",{"session_id":sid,"actions":json.dumps(actions)})
print("plan ok",pl["ok"],[ (s["kind"],s["obligation"]["deadline_tick"]) for s in pl["steps"]])
c=tool("fortress_commit",{"session_id":sid,"plan_digest":pl["plan_digest"]})
print("commit", [a["state"] for a in c["actions"]], "recommend:", c["agent_turn"]["recommendations"][0]["tool"], c["agent_turn"]["recommendations"][0]["arguments"])
for i in range(30):
    w=tool("fortress_wait",{"session_id":sid,"max_game_ticks":100})
    if w["open_actions_remaining"]==0: break
print("waits",i+1,"tick",w["game_tick"],[a["state"] for a in w["polled_actions"]])
t=tool("fortress_query",{"session_id":sid,"mode":json.dumps({"mode":"terrain","min":[0,2,10],"max":[5,6,10]})})
print("\n".join(t["levels"][0]["rows"]))
b=tool("fortress_query",{"session_id":sid,"mode":json.dumps({"mode":"entities","kind":"building"})})
print("building:",b["rows"][0]["label"],b["rows"][0]["fields"]["construction_stage"])
p.stdin.close(); p.wait(timeout=10)
