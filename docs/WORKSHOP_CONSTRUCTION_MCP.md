# Original Carpenter workshop development slice

Original work item: `dfmcp-reality-wiring-proof-4bf.14`. This distinctly versioned
workshop/1.20 profile is unadmitted development source. It does not inherit R28,
R29, furniture/1.19, native R1, live-game or production qualification.

`dfmcp-workshop-v1-20-dev-server` registers the existing eleven modern MCP tools.
Operations use dotted semantic tags; directly callable MCP tool names use
underscores, for example `fortress_open_session` and `fortress_commit`.
No tool accepts a path, endpoint, native address, arbitrary command or Lua.

The operator pins one candidate log and one fixed 3x3 Carpenter workshop center.
`open_session(mode="control")` and `observe` expose the actual workshop/1.20
source, all nine floor witnesses, exact material, bounded complete worker census,
eligibility blockers and a witness-bound material handle. This is bounded
candidate inspection, not unrestricted inventory discovery. The material must
be outside the footprint. Enabled CARPENTER labor, labor validity, job
availability, visibility, adulthood, sanity and walk-group connectivity are
independent observed worker requirements. They do not promise assignment or
scheduling. A walk-group match is not a full route proof.

Native facts have their own references. No canonical world adapter or join to
operations/1.3/1.4 exists for this profile; Agent Turns keep a null canonical
anchor and explicitly unestablished world continuity. Verified local custody
does not turn historical native facts into current fortress observations.

## Operator configuration

The explicit process environment is:

```text
DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20=1
DFMCP_WORKSHOP_MCP_CONFIG=<closed JSON below>
DFMCP_WORKSHOP_TOKEN=<operator-held 32..256-byte native credential>
DFMCP_WORKSHOP_ALLOW_BUILD=1
```

Production launch/ticket/protocol state and other DFMCP profiles are refused.
The public wrapper rejects `DFMCP_ADMITTED_BRIDGE_PROTOCOL` with any present
value before entering stdio. It never consumes tickets or supplies production
admission provenance. The workshop session namespace is distinct and process
scoped. Build permission is checked again before native preparation/commit.
Authenticated recovery, retirement and progress do not require build permission.

The config is environment JSON, not a file read by the server:

```json
{
  "schema": "dfmcp.workshop-mcp-development/1",
  "directory": "/private/workshop",
  "endpoint": "127.0.0.1:5000",
  "initialize": true,
  "acknowledged": null,
  "world_folder": "<exact operator-accepted folder>",
  "site_id": 0,
  "df_version": "<exact observed version>",
  "dfhack_version": "<exact observed version>",
  "generation": 1,
  "map_dimensions": [64, 64, 4],
  "item_id": 42,
  "center": [15, 15, 2],
  "scope": [0, 0, 0, 63, 63, 3],
  "protected": [],
  "authority_expires_tick": 20000,
  "lease_ticks": 1200,
  "checkpoint": "required"
}
```

Replace every source, clock and material value with independently accepted
operator facts; these example numbers are not live evidence. Scope and protected
regions use inclusive `[x1,y1,z1,x2,y2,z2]` bounds. The exact log and footprint
must fit the host scope and avoid protected regions. At least one observed
eligible carpenter must lie in scope. An exclusive process-local lease covers
the affected scope; its original lifetime is not renewed by repeated calls.

The existing private-file primitive requires a canonical absolute directory
owned by the effective user, mode `0700`, and regular single-link `0600` files
with descriptor/identity checks and exclusive locks. The server creates neither
the directory nor an alternative path. `initialize=true` requires genuinely new
`workshop.ack` and `workshop.journal` custody. Existing, empty, torn or missing
recovery files are never silently reinitialized.

`checkpoint="required"` refuses preparation because this slice has no checkpoint
provider. The already authorized disposable-fortress experiment may instead set
`"disposable-fortress-no-checkpoint"` before opening. MCP arguments cannot grant
or change this exception. No order activation, manager state or validated flag
is forced by this profile.

## Original plan and one attempt

1. Call `fortress_open_session` with `mode="control"`. Review the complete source
   and returned acknowledgement. Default wall work is 5 seconds. Explicit
   `max_wall_millis=60000` permits the existing native prepared-connection review
   lifetime; it cannot extend that original connection or native TTL afterward.
2. Call `fortress_plan` with the session ID and a closed JSON `request` containing
   `kind="carpenters"`, the exact `expected_observation` and observed `material`
   handle. Preparation is side-effect free with respect to game construction.
3. Review the entire original plan, source, material/worker/footprint evidence and
   `review_seal`. Preserve the returned acknowledgement outside both private
   custody files. The native plan and receipt use separate 1.20 domains.
4. Call `fortress_commit(session_id, review_seal, confirmed=true)`. It uses the
   original prepared connection, repeats source and authority/lease checks, then
   synchronizes `DispatchStarted` in `workshop.journal` and an independent
   acknowledgement in `workshop.ack` before sending the one native writer.
5. Inspect the original insertion receipt. `Placed` certifies historical
   stage-zero building/job registration and selected Hauled item/reverse links,
   not completed construction. `building_occupied_at_anchor` is the capture's
   observed nonempty building occupancy; only the insertion oracle independently
   certifies the Planned mask.
6. Call `fortress_wait` for one bounded original-key reconciliation or a fresh
   `ReadWorkshopProgress` sample. The original subtype, geometry, material,
   maximum stage and source/generation/clock continuity must match, stage must
   equal maximum, and no ConstructBuilding job may remain. Completion does not
   require the original log to remain free or a worker to remain idle.

Repeated commit cannot write again. Query/cancel-cleared or different prepared
connections cannot commit. A failed entry check, invalid work budget, source
call or custody operation withdraws retained observation and preparation
permission. Restoring old configuration or a valid context cannot revive it.
Unknown outcomes remain tied to the original key. No new key or replan can make
them green. Cancellation retires unattempted preparation or queries dispatched
work; it never deconstructs a building or erases history.

## Crash recovery and freshness

Preserve both original files and the externally held
`{"journal_id":"...","head":"...","frames":...}` acknowledgement. For a new
process set `initialize=false` and `acknowledged` to that pin. `mode="recover"`
permits original-key query/retirement and sampling; `mode="offline"` only
inspects local historical custody without native calls or credential reads.
Offline inspection does not append an acknowledgement or mutate either file.
`cancel(scope="session")` releases local sockets and locks even after fencing,
without cancelling game effects or deleting custody.

The acknowledgement file is independently synchronized before dispatch. Thus
rolling the journal back to an older valid prefix is rejected even if the MCP
reply was lost and the native retained table has disappeared. A checked
descendant can resume evidence; reopen never restores dispatch permission.
Missing, corrupt or rolled-back acknowledgement custody fails closed. This is
local crash/rollback custody, not distributed consensus or protection against
rolling back both stores and all external acknowledgements together.

The journal preserves consumed clock/generation high-water and a sticky
original-continuity break through full replay. `G -> G+1 -> G`, same-generation
clock regression and new plans below consumed progress cannot prove completion
or obtain a new preparation. Optional `recovery_generation` pins an explicitly
accepted later native incarnation for recovery; it cannot establish continuity
of the original building after restart/restore. Unknown and observed absence
remain distinct.

Only a newly acquired progress sample is labelled fresh. Repeated wait after
immutable historical completion does not relabel its cached result fresh.
Failures withhold current completion. Full and bounded-overflow responses use
the same Agent Turn spine, retain compact original hazards and recovery
references, and never fabricate a canonical world anchor.

## Separately reviewed game progress

This consumer does not control time. The existing standalone
`dfmcp-live-run-dev-server`, documented in
[BOUNDED_SIMULATION_MCP.md](BOUNDED_SIMULATION_MCP.md), can provide a separately
authorized finite run/1.13 interval without furniture/work-order/workforce goals
or manager flags. Its independent eleven-tool session follows
open-control, observe, plan with original idempotency key/expected witness,
review exact digest, confirm commit, and wait on that same key/digest. Limits
remain at most 1200 ticks and 60000 native wall milliseconds per reviewed run.
It is not the earlier work-order-linked clock.

Before gameplay, the exact native artifacts and same-game endpoint/owned process
identity must be independently established. Protocol 1.13 does not name a
fortress or provide cross-plugin atomicity or a global controller fence. A run
receipt alone proves neither workshop completion nor construction causality.
After a separately reviewed interval, take fresh workshop/1.20 evidence of the
original building and retain unknown causality where not independently proved.
No game start, unpause, plugin installation or save/config mutation is authorized
by these source instructions.

The original `.14` also requires untouched real-game workshop proof and finite
furniture output. Manager/activation and finite-bed prerequisites remain explicit
gaps under that original item. Operator UI activity is not MCP capability.

## Evidence gates

`architecture/workshop_construction_v1_20.json` and
`scripts/check_workshop_construction.py` bind the exact registered source and
consumer boundaries. Both mandatory verification commands run the static gate,
required-input negatives and compiled native mutants; Cargo discovers the
workshop Rust and actual stdio integration tests in debug and release.
The native harness uses SDK/protobuf doubles. Its raw compiler/program streams,
exits and reports are component evidence; a passing run is not real SDK or R1.
Historical vectors remain unchanged; a separate continuous-absence vector
distinguishes actual absence from a reset ID horizon.

Real SDK diagnostics, both clean frozen-source qualification commands, final
native artifact review and untouched gameplay are separate evidence classes.
Only the independent verifier may close earned criteria. Source or focused
component execution alone does not close this item or grant admission.
