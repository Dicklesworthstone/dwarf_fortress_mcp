# Original Carpenter workshop development slice

Original work item: `dfmcp-reality-wiring-proof-4bf.14`. This distinctly versioned
workshop/1.20 profile is unadmitted development source. It does not inherit R28,
R29, furniture/1.19, native R1, live-game or production qualification.

`dfmcp-workshop-v1-20-dev-server` registers the existing eleven modern MCP tools.
Operations use dotted semantic tags; directly callable MCP tool names use
underscores, for example `fortress_open_session` and `fortress_commit`.
No tool accepts a path, endpoint, native address, arbitrary command or Lua.

The operator pins one fixed 3x3 Carpenter workshop center. The original profile
pins a candidate log; the opt-in Tree material profile below discovers new log
candidates from acknowledged Tree/1.21 custody.
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

## Discover material from an acknowledged Tree original

The opt-in material profile removes the need to know a log ID before harvesting.
It imports one real Tree/1.21 original and its independently acknowledged
history, reads current ground-WOOD supply for that original, and offers at most
64 opaque material handles. It then independently qualifies the selected log
with Workshop/1.20. Both native wire generations, the eleven public MCP tools
and the one-attempt workshop journal remain unchanged.

Finish or reconcile the Tree operation and close its session before importing
its custody. The importer holds the original `tree-harvest.journal` and
`tree-harvest.ack` exclusively in Offline mode. It rechecks both exact files
around native discovery and qualification reads without appending, sampling or
advancing Tree custody. The donor must have an independently acknowledged
original designation receipt, resolved history, no pending sample and intact
original source continuity. An arbitrary progress capsule, caller-provided
native item ID, unacknowledged journal or replacement Tree plan cannot serve as
a donor.

Keep the workshop environment above and add:

```text
DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL=1
DFMCP_TREE_HARVEST_TOKEN=<operator-held read-only discovery credential>
```

Use `schema="dfmcp.workshop-tree-material-mcp-development/1"`,
omit `item_id`, and add the following field to the common workshop
configuration:

```json
{
  "tree_material_source": {
    "directory": "/private/tree",
    "endpoint": "127.0.0.1:5001",
    "generation": 7,
    "root": [15, 15, 2],
    "scope": [0, 0, 0, 63, 63, 3],
    "read_scope": [0, 0, 0, 63, 63, 3],
    "acknowledged": {
      "journal_id": "<externally preserved Tree journal digest>",
      "head": "<externally preserved Tree head digest>",
      "frames": 4
    }
  }
}
```

These values are examples, not observed source identities. The Tree directory
must be distinct from workshop custody and satisfy the same private-file rules.
Its root, scope, endpoint and generation must match the retained Tree binding.
The common fortress folder, site, map dimensions, DF version and DFHack version
must agree across the imported history and both independently authenticated
native sources. Tree and Workshop generation numbers are independently pinned;
their numeric values need not match. `read_scope` must explicitly cover the
complete loaded map because the Tree supply RPC inspects that domain. The
workshop's mutation scope and protected regions still constrain the selected
log, footprint and eligible worker.

The additional opt-in permits only the Tree read credential. It does not permit
Tree designation configuration, mutation opt-ins, other development profiles or
production launch state. The original workshop schema continues to require its
fixed item and rejects the donor configuration. The new profile defaults to
256 MiB of bounded request work to cover repeated donor custody verification;
the original profile retains its 128 MiB default. This is a work allowance,
not a native payload or output limit.

1. Open a control session or call `fortress_observe`. The result contains
   `tree_material.witness`, the exact original Tree key/digest and ACK,
   current source facts, original/current item horizons and sorted bounded
   offers. Each offer contains a `material_handle`, observed log ID,
   position and material. IDs are evidence for review; they are not selection
   arguments. The `host_selection_allowed` flag checks only operator scope,
   protected regions and the fixed footprint. The initial capture prefers the
   first such candidate. If that captured log is ineligible, a recommendation
   may select another allowed handle for fresh independent qualification; the
   flag and recommendation do not assert native eligibility.
2. Follow the returned recommendation or call `fortress_plan` with this
   closed JSON request:

```json
{
  "kind": "carpenters_from_tree_supply",
  "expected_observation": "<tree_material.witness>",
  "material": "<one tree_material.offers[].material_handle>"
}
```

3. Planning resolves the exact retained handle, rereads Tree supply, and refuses
   loss or changes in the selected log's identity, position or material. A
   fresh Workshop/1.20 capture must agree on fortress, dimensions and paused
   game tick, and independently prove material, footprint and worker
   eligibility. Source regressions, expired authority or changed donor custody
   withdraw the selection before preparation.
4. Review and commit the resulting original workshop plan through the same
   existing `review_seal` flow below. Its durable Intent embeds the exact
   selected Workshop capture. After preparation the workshop owner releases
   the donor; it cannot replace the selected log or renew the Tree goal.

The discovery is a fresh material-readiness observation. It does not attribute
logs causally to the Tree operation, prove the original harvest goal within its
deadline, reserve a log, establish a canonical world anchor, or claim an atomic
cross-plugin snapshot. Those distinctions are explicit in the response.
Ordinary workshop fresh revalidation still guards the eventual writer.

Recover and offline modes derive selection exclusively from the original
workshop Intent. They do not open Tree files, read the Tree credential, contact
the Tree endpoint or rerun material discovery. Preserve the original workshop
journal, ACK and external pin, keep the chosen material policy in operator
configuration, and set `initialize=false` as described below. Loss of a commit
reply remains attached to the same original key and log.

## Original plan and one attempt

1. Call `fortress_open_session` with `mode="control"`. Review the complete source
   and returned acknowledgement. Default wall work is 60 seconds, the existing
   maximum; it cannot extend that original connection or native TTL afterward.
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
   An expired connection or failed fresh read before that latch returns its
   typed cause with no dispatch record. Local permission remains consumed;
   later post-latch failures remain indeterminate.
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

## Hand off an original workshop to production

The adapter exposes `workshop_construction::production_origin` so an importing
production host can require the exact original Carpenter workshop to be complete
before its own effects. The [combined production host](COMBINED_PRODUCTION_MCP.md#require-an-exact-completed-original-workshop)
uses this embedding API for closed operator configuration, an immutable original
plan association and fresh checks at every order preparation and dispatch boundary.
It adds no MCP tool or native method and creates no canonical workshop fact.

Close the workshop session before importing its exclusive private custody. Open
both `workshop.journal` and `workshop.ack` in Offline mode with the independently
retained acknowledgement, then call `WorkshopJournal::production_origin` with
the exact original key and plan digest. The importer requires the resolved
original `Placed` receipt and rejects an unacknowledged journal, valid prefix
rollback, another protocol's acknowledgement or retained source discontinuity.
A historical completion sample is optional because it cannot establish current
readiness. The original receipt supplies the exact building and material IDs;
the importing request does not select an arbitrary native building.

`WorkshopBinding::from_source_pins` constructs independently accepted endpoint,
fortress, generation, dimensions and software pins without inventing a capture.
These pins do not issue source authority. The host still authenticates and
qualifies the actual native source under its current Query and Observe grants.

The non-cloneable `WorkshopProductionOrigin` retains stable descriptor bytes and
a digest covering the exact binding, acknowledged donor pin, original receipt
and retained floor seed. A host can bind these bytes to its immutable original
production plan. `verify` rechecks local historical custody without credentials,
native contact or donor writes. `refresh` independently checks that custody
around one fresh `ReadWorkshopProgress` for the original building. It requires
the original source incarnation, exact geometry/material/maximum stage, complete
construction with no remaining ConstructBuilding job, and the host's freshly
observed paused tick. It does not call the donor's `sample` method, which may
return already achieved historical completion.

Every decoded tick, generation and building allocator horizon is consumed before
later authority, custody or host-tick checks can fail. Complete retained history
is checked at import, so an older sample's higher allocator horizon cannot be
hidden by the latest sample. Source resets, original absence or identity
contradiction permanently fence fresh qualification. An incomplete current
workshop can be sampled again while preserving its original identity and floors.
The host must retain one origin owner throughout the session. Connection or
handshake uncertainty before `refresh` must call its one-way `fence`; withdrawal
requires no surviving capability grant and cannot be reversed by Query.
Immediately after a successful handshake, the host calls
`retain_source_manifest` before any fallible budget or deadline check. `refresh`
also consumes that already decoded manifest before its local preconditions.
This operation only raises the generation floor or fences a mismatched source;
it performs no I/O and cannot grant authority or fresh completion. A matching
later manifest cannot clear an earlier fence.

Recovery has two distinct inputs. The immutable donor descriptor binds the
original workshop. The separate `retain_recovery_floor` API consumes the
importing host's externally retained tick, native generation, next-building
counter and sticky fence. It rejects lower or invalid values and never clears
a fence. These caller-held recovery floors are rejection evidence, **not a
local durable acknowledgement of fresh progress reads**. The importer does not
append to the donor journal or ACK. A host must preserve those consumed floors
across handoff or recovery, rather than remint an owner from old donor history.
On a new owner, donor custody supplies only its immutable retained seed. It
cannot independently detect rollback of a later externally held fresh-read
floor that still satisfies that seed, including a floor in complete importing
host recovery. An immutable host plan association prevents replacing the donor
policy; it does not durably acknowledge subsequent read floors.

Each operation prepays its complete logical work and keeps one shrinking wall
deadline of at most 60 seconds:

| Operation | Required byte allowance |
| --- | ---: |
| Import complete original custody | 96 MiB |
| Verify exact local custody | 24 MiB |
| Refresh between both custody checks | 64 MiB |

Native connection setup has a separate host-owned reservation before contact.
The narrow `open_private_association_storage` helper lets that host create an
immutable association in at most 64 KiB of the existing private-file custody.
Creation requires Query and Plan; reopening uses Offline mode. It supplies no
association codec, automatic initialization, repair or publication guarantee.

Qualified completion applies to the exact fresh read. Every later effect
boundary must refresh it again. It does not prove manager readiness, available
production material, worker scheduling, cross-plugin atomicity, global restore
continuity, finished furniture or causal production by an order. The original
workshop history and deadlines are unchanged. Current production control and
original-goal proof still belong to their separately authorized owners.

## Separately reviewed game progress

The optional independently authorized [workshop clock](WORKSHOP_CLOCK.md) now
connects finite Run/1.13 intervals to this exact original Carpenter in the same
eleven-tool session. Its default tick allowance is zero. Each interval needs a
separate plan, exact review and confirmed commit, fresh durably acknowledged
original-workshop guards, and its own Run journal/recovery identity. A settled
`fortress_wait(scope="clock")` acquires fresh original-building progress before
reporting completion. Unknown source continuity never grants another interval.

The existing standalone `dfmcp-live-run-dev-server`, documented in
[BOUNDED_SIMULATION_MCP.md](BOUNDED_SIMULATION_MCP.md), remains available as an
independent clock consumer. Both paths retain the native limits of at most
1200 ticks and 60000 wall milliseconds per reviewed run. They do not establish
manager state, activate orders or produce furniture by themselves.

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
workshop Rust and actual stdio integration tests in debug and release. The
material-handoff wrapper runs both actual development binaries against joined
Tree and Workshop protocol doubles, including forged/stale selection,
independent material/source drift, lost reply and credential-free offline
recovery scenarios. The machine contract records these cases separately from
the unchanged native component scenarios.
The native harness uses SDK/protobuf doubles. Its raw compiler/program streams,
exits and reports are component evidence; a passing run is not real SDK or R1.
Historical vectors remain unchanged; a separate continuous-absence vector
distinguishes actual absence from a reset ID horizon.

Real SDK diagnostics, both clean frozen-source qualification commands, final
native artifact review and untouched gameplay are separate evidence classes.
Only the independent verifier may close earned criteria. Source or focused
component execution alone does not close this item or grant admission.
