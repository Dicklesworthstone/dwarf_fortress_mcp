# Vacant Manager appointment (unadmitted 1.22 development)

This is one bounded prerequisite for a separately reviewed finite production
order: appoint the observed current Expedition Leader to an existing vacant
`MANAGER` assignment. It uses the existing eleven modern MCP tools. There is no
role selector, reassignment, order activation, labor change, item flag change,
clock control, Lua, command, keyboard or native-address input.

Ownership remains `dfmcp-reality-wiring-proof-4bf.14.2` under original `.14`.
Only the independent verifier closes those items. Source and component tests
are not a live appointment or bed-chain proof. Protocols 1.0/1.1, the frozen
production runner map and the empty compatibility registry remain separate.

## Start one configured development process

The standard executable accepts:

```text
dwarf-fortress-mcp serve-gameplay-dev manager
```

The dedicated component/evidence binary is `dfmcp-manager-v1-22-dev-server`.
Both enter the same guarded runtime. Set these host environment variables:

- `DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22=1`;
- `DFMCP_MANAGER_TOKEN`: the native plugin's 32–256-byte loopback secret;
- `DFMCP_MANAGER_ALLOW_APPOINT=1` for new control only;
- `DFMCP_MANAGER_MCP_CONFIG`: the closed JSON configuration below.

Every other `DFMCP_*` variable is refused, including production tickets,
protocol markers, compatibility/floor/receipt state and other development
profiles. Credentials and configuration are host state, never MCP arguments.
The plugin is `dfmcp_manager_v1_22`, package `dfmcp.manager.v1_22`.
Its fixed native methods are `Handshake`, `ReadManager`, `PrepareManager`,
`CommitManager`, `QueryManager`, `CancelManager`.

Configuration fields are:

| Field | Contract |
|---|---|
| `schema` | `dfmcp.manager-appointment-mcp-development/1` |
| `directory` | Absolute private custody directory, exact mode 0700; fixed journal/ACK filenames, exact mode 0600 |
| `endpoint` | Canonical numeric IPv4 loopback address with nonzero port |
| `initialize` | `true` only for new empty custody in Control mode |
| `acknowledged` | `null` on initialization; otherwise retained `{journal_id,head,frames}` |
| `world_folder`, `site_id` | Exact observed fortress identity |
| `df_version`, `dfhack_version`, `generation` | Exact native software and incarnation pins |
| `map_dimensions` | Three positive dimensions, each at most 32768 |
| `unit_id`, `histfig_id` | Host-offered exact identity; native eligibility also requires the current Expedition Leader |
| `read_scope`, `scope` | Both exactly `[0,0,0,width-1,height-1,depth-1]`; cropped spatial authority cannot appoint a fortress-wide official |
| `authority_expires_tick` | Absolute game tick beyond the reviewed observation |
| `lease_ticks` | 1–1200; conservative whole-fortress local exclusive lease |
| `checkpoint` | Exactly `disposable-fortress-no-checkpoint` |

All fields are required except the nullable `acknowledged`. Unknown fields fail.
Every native plan edge authorizes at the maximum caller tick, original plan
capture tick and retained connection high-water tick, including lookup and
retirement. Validated observations advance that connection floor. A caller
cannot revive expired authority by supplying an older context or by skipping
observe on the preparing connection. Dispatch advances its one shrinking
work context to the retained journal tick before both external guards and the
writer, so injected lease checks see the same authority frontier. This changes
no wire bytes, wall/byte allowances or post-dispatch uncertainty rule.

The local lease coordinates this process only; it is not a global lock on DF,
other controllers or people. Plugin-byte/source provenance must be verified by
the separate exact build/trial evidence. Endpoint/version/incarnation pins are
not compatibility admission or an artifact attestation.

## Observe, prepare, confirm once

1. `fortress_open_session(mode="control")` returns the host-offered citizen,
   exact native capture and eligibility, software/source identity and independent
   ACK. Opening performs no appointment. Defaults are 5 seconds wall time,
   256 MiB logical byte work, one action and zero game ticks. The ceiling is
   60 seconds/512 MiB with a bounded 512 KiB complete response.
2. Follow its `fortress_plan` recommendation. The closed request string contains
   `kind: "appoint_vacant_manager"`, `expected_observation` and `deadline_tick`.
   The original absolute deadline is at most 12000 ticks beyond the capture and
   no later than the host authority expiry; appointment still advances no ticks.
3. Review the returned original source, citizen/HF, position ID and vector index,
   assignment ID and vector index, retained raw flags, intended POSITION link,
   single history event and exact `review_seal`. The native preparation expires
   after 60 seconds; replay never renews it.
4. `fortress_commit(session_id, review_seal, confirmed=true)` consumes that
   original connection's permission once. The complete write-ahead journal and
   independent ACK synchronize before the native writer. Native revalidation,
   transaction and readback occur under `CoreSuspender`.
5. Inspect the returned original receipt. `appointment_proved=true` requires
   all projected assignment/HF/history writes, unchanged tick and both
   `Units::getUnitByNobleRole("MANAGER")` and `getNoblePositions` proofs.
   Repeating that exact terminal review returns retained evidence without
   another writer or event. A finite-bed order remains a separate configured
   profile, observation, preparation, review and confirmation.

The initial eligibility is deliberately narrow: paused ordinary fortress UI,
active sane adult citizen, reciprocal unit/HF/nemesis identity, group/civ
membership, current Expedition Leader, an ordinary eligible MANAGER definition,
one ordinary active vacant assignment and no competing HF position link or
getter result. Target squad/site/building/vassal fields must be -1, temp zero
and claims empty. The assignment's low flag nibble must equal 1; unknown high
bits and all other captured fields are preserved byte-for-byte.

The transaction sets `histfig`, `histfig2`, and the position-definition cache,
appends the selected HF's complete POSITION link, and appends exactly one
`history_event_add_hf_entity_linkst` with the next event ID and current year/tick.
Event ID is distinct from vector index; position ID is distinct from vector
index. Appointer and promise remain -1. The supported generated allocation
uses `df::allocate`; the event constructor's one-zero-byte flags are preserved.
Allocations, vector capacity growth, constructor checks and repeated witness
checks precede world writes where possible. Partial failure stays uncertain;
there is no speculative rollback or retry.

## Recovery and evidence packaging

Preserve `manager-appointment.journal`, `manager-appointment.ack` and the last
external ACK together. Reopen with `initialize=false`, that ACK, the unchanged
host source/candidate pins and `mode="recover"` or `mode="offline"`. Restored
bytes never grant a fresh appointment permit. `fortress_wait` performs one
original-key lookup/reconciliation and never dispatches or advances time.
Missing native history, source changes and incomplete poststate remain
unresolved. `fortress_cancel(scope="goal")` retires only an unattempted
preparation; attempted work uses original-key reconciliation. Session cancel
releases sockets/locks and preserves both files. Checkpoint/restore refuse.

Every success/error keeps the Agent Turn spine, original key, plan/review seals,
source/version/incarnation, dispatch status, uncertainty and ACK references.
`native_projection_anchor` binds the declared projection's complete fortress
identity, generation, sequence, tick and SHA-256. Canonical world/MVCC continuity
is explicitly unestablished; the native generation is not a canonical epoch.
Historical proof does not establish present scheduling or all-game-field
identity. Raw canonical captures are losslessly detached into 24000-byte hex
blocks, with ordered content indices, offsets, lengths and whole/chunk SHA-256.
Verify every block before treating the complete projection as evidence.
The unchanged 512 KiB product ceiling reserves 4096 bytes for the largest
admitted numeric JSON-RPC ID lexeme; 256 is only the encoded string-ID bound.
The focused boundary test uses the actual pinned request/response codec and
synthetic valid MCP text blocks. It does not assert that a natural native
appointment packet reaches that interval. Existing representative capture,
chunk-hash and ACK preservation assertions remain in place.

## Focused checks and remaining evidence

`architecture/manager_appointment_v1_22.json` lists exact source inputs.
`python3 scripts/check_manager_appointment.py` is the named static generation
inventory gate; it detects missing/stale inputs and unregistered wire/CLI
routes. Its receipt cannot certify Rust, SDK or game execution. Remove the
gate with removal of this feature. Its consumer is the component build and the
existing repository static/test batches.

The eight adapter modules contain codec, native-vector, real-file custody and
joined-TCP tests. Corrected native component vector bytes come from Root's
flags-green cut; they are explicitly component output, not live fixtures.
`tests/stdio/manager_appointment.py` drives five actual Rust subprocess cases
through either the dedicated binary or standard CLI, using joined protocol
doubles. It checks one complete appointment, unchanged tick, replay without an
extra event/writer, offline custody, lost-reply/crash recovery, changed vacancy,
identity/source/epoch, closed candidate/role arguments, confirmation, budgets
and production-state refusal. The Rust wrappers assert exact test counts and
retain stdout/stderr separately.

Root reported actual SDK configure/build success on its attempt 4; that is
native source-build evidence only, not a live appointment. Adapter cut 2 had an
actual E0603 compile failure and zero executed tests. Cut 3 fixes that visibility
error and full-fortress scope; execution remains pending at this writing.
MCP/direct CLI execution, full exact-source Rust qualification, disposable-game
Manager proof and the bed chain require independent execution. No admission,
release or feature closure is claimed here.

Independent Terra source review subsequently identified stale-caller-tick
checks at public adapter boundaries: a direct prepare could use an expired
capture-time grant, and injected dispatch guards/writers received the older
caller tick. No unauthorized actual MCP effect was shown. Adapter cut 4
normalizes those contexts and retains observed high-water time. Three paired
component regressions cover direct prepare without prior observe, recovery
edges after a later observation, and a real lease check that refuses before
any writer/commit frame while the permitted pair commits once. These new Rust
regressions remain unexecuted by the author pending Root's direct build.
