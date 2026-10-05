# Original-room excavation handoff

`scripts/survey_rooms.py --emit excavation-handoff` exports a complete
`dfmcp.room-excavation-handoff/1` artifact from the command's one map/1.5
observation. It retains the original canonical RoomPlan, raw capture bytes,
map endpoint/manifest, survey digest, and exact remaining-wall blueprint.
The default survey and standalone remaining-blueprint exports are unchanged.

```sh
# Use the existing isolated map-read environment; no dig credentials here.
python3 scripts/survey_rooms.py --plan-file room-plan.json \
  --emit excavation-handoff > room-excavation.json
```

Check exit status before consuming redirected output. Blocked, empty or lost
observations emit no handoff. Decode regenerates the original intention and
reruns the complete survey; changing a residual or survey digest and recomputing
its outer hash does not pass. Already-dug floors and required walls never become
residual targets. All original furniture constraints and exclusions survive.

This artifact has a separate 256 KiB bound, including retained raw evidence.
The command still reserves the complete 128 KiB survey report and uses one
shared work/wall/native budget; the handoff performs no additional native read.
Depth is bounded to 12, duplicates and nonfinite JSON are rejected, hexadecimal
and all serialized bytes are canonical. Live authorization is rechecked after
serialization. The original map and subsequent dig generations are independent
namespaces; only explicit endpoint, fortress, dimensions, software and monotonic
source-clock checks may be applied across them. These selectors cannot detect
an unseen same-tick restore or attest that an exported file was natively acquired.

The artifact is reproducible evidence, not mutation authority or an effect
inventory. Never use it to replace an unresolved dig batch or mint retry keys.
Fresh native review, operator confirmation, original-key custody and recovery
remain necessary. This first increment exports and validates the handoff; its
durable digging integration is a separate increment. Full-room terrain goals,
furniture completion and native room assignments remain distinct from finishing
the residual excavation.

Executed on 2026-10-05: `PYTHONPATH=scripts:tests python3 -m unittest
 test_room_excavation_handoff -v` passed 17 methods (12 core and 5 CLI/TCP).
Tests include partial and multi-level work, complete 32-slot intentions, source
checks with intentionally different map/dig generations, rehashed substitutions,
blocked/empty work, malformed bytes, shared budgets, final revocation and the
unchanged legacy exports. TCP tests execute the actual existing map client and
CLI against the joined synthetic peer; native DFHack, native digging, Rust/MCP,
live fortress and full-repository qualification were not executed.

Owning beads df-dfhack-bridge-plane-c-pic.3/.4/.5 (WP-05/WP-10) remain open.
Production admission, native protocols and existing journals are unchanged.
