# Original mining development runtime

This isolated runtime joins one canonical `DesignateDig { mode: Mine }` plan to
native designation, exact restart recovery and fresh proof of the original
excavation goal. It supports one rectangle, **1..8 tiles wide and 1..8 tiles high
on one z-level**, one original goal and one private custody bundle per session.
The dedicated server uses the frozen eleven-tool MCP surface.

This remains an **unadmitted development profile**. Source and focused component
tests do not establish a complete FastMCP executable, native DFHack build, live
fortress campaign or compatibility admission. The exact validation evidence is
recorded in [implementation status](../IMPLEMENTATION_STATUS.md).

## What a successful result establishes

| Claim | Meaning |
| --- | --- |
| Historical designation | The original dig/1.16 key returned a verified `Designated` receipt. It records mining flags, not excavation completion. |
| Current excavation | A newly acquired complete paused spatial/1.8 capture shows every original target cell as a known dry floor with no remaining dig designation. Hidden, missing, unsupported, wet or still-designated cells cannot prove completion. |
| Original goal achievement | The original postconditions, terminal and failure predicates, absolute deadline, polling cadence and stable-sample requirement are evaluated under independently supplied evidence policy. |
| Historical first achievement | A retained qualified sample previously satisfied the original goal. This history does not establish current terrain after restart or contradiction. |

Each current sample is linked to the exact retained native receipt, original
plan archive, target rectangle and source identity. Exact local native-journal
and association checks bracket the fresh spatial read. This is not a new dig
query RPC bracket or a claim about which dwarf performed the mining. Native
receipt custody and independently authorized terrain truth remain distinct.

Partial canonical chunks preserve unobserved cells explicitly. The source's
pinned identity or stored policy fingerprint cannot create evidence authority.
Unknown terrain cannot become a proven floor, an effect input or a safe path.

## Operator setup

The source entry point is:

```bash
cargo run --locked -p dfmcp-mcp --bin dfmcp-live-semantic-dig-dev-server
```

The build environment needs the repository's exact pinned dependencies. This
command identifies the entry point; it is not a full-server qualification claim.
Private filesystem custody is implemented for Linux x86_64 and aarch64.

Only these `DFMCP_*` environment names are accepted:

| Variable | Value |
| --- | --- |
| `DFMCP_ALLOW_UNADMITTED_SEMANTIC_DIG` | Exactly `1`. |
| `DFMCP_SEMANTIC_DIG_CONFIG` | The bounded JSON object below, supplied as the environment value itself. |
| `DFMCP_SEMANTIC_DIG_ALLOW_DESIGNATE` | Exactly `1` to enable initial designation or explicit retirement of an existing preparation; omit for read-only recovery. |
| `DFMCP_SEMANTIC_DIG_SPATIAL_TOKEN` | Credential for the independently pinned spatial/1.8 source. |
| `DFMCP_SEMANTIC_DIG_DIG_TOKEN` | Credential for the dig/1.16 source. |

Credentials must contain 32..256 UTF-8 bytes and no NUL. Offline reopening needs
no native credentials. Endpoints, credentials, paths, checkpoint policy and source
pins are operator configuration and cannot be supplied through MCP arguments.
Production admission environment and other `DFMCP_*` names are rejected. Runtime
cancellation, configuration, credentials and shrinking deadlines are rechecked
at bounded native I/O boundaries.

The following values are **illustrative**, not discovered source identities.
Replace the world, software versions, source generations, dimensions and scope
with independently obtained values for the exact running sources:

```json
{
  "schema": "dfmcp.semantic-dig-development/1",
  "directory": "/absolute/private/mining-goal",
  "endpoint": "127.0.0.1:5000",
  "initialize": true,
  "sources": {
    "kind": "operator-development-source-acceptance/1",
    "world_folder": "region1",
    "site_id": 1,
    "df_version": "OPERATOR_OBSERVED_DF_VERSION",
    "dfhack_version": "OPERATOR_OBSERVED_DFHACK_VERSION",
    "spatial_generation": 7,
    "dig_generation": 3,
    "map_dimensions": [96, 96, 32]
  },
  "scope": [0, 0, 4, 15, 15, 6],
  "protected": [],
  "checkpoint": "disposable-fortress-no-checkpoint"
}
```

Generation counters belong to independent sources and need not match. The
endpoint must be canonical numeric IPv4 loopback with a nonzero port. Scope and
protected cuboids use inclusive `[min_x,min_y,min_z,max_x,max_y,max_z]` coordinates
inside the pinned map; at most 32 protected cuboids are accepted. The target's
full one-cell 3D halo and affected **whole 16x16 native map blocks** must fit the
operator scope. The write lease covers shared blocks, not just selected tiles.
Protected areas are checked against those writes. Hidden neighbors are refused.

The scope also bounds each complete spatial observation. Its terrain, citizens,
items, buildings and jobs must fit the operation's actual entity allowance.
Large or fragmented observations can be refused instead of silently truncated.
The first source handshake requires a valid small bootstrap region inside a
complete block and its halo in the configured scope.

Checkpoint policy defaults to `required`. There is no checkpoint provider in
this profile, so new planning/control requires the explicit disposable-fortress
exception **before the original plan is sealed**. Existing required checkpoints
cannot be removed from a plan. Retirement of an existing preparation requires
current designation authority, without a new checkpoint exception or write lease.

The custody directory must already exist as a real owner-private `0700`
directory. The runtime owns fixed, regular, single-link `0600` files:

- `dig.journal`: exact native plan, key and single-attempt effect coordination.
- `semantic.associations`: immutable original-plan/native association.
- `original.plan`: lossless original plan and archive identity.
- `goal.history`: bounded samples, proof publication and historical achievement.

Symlinks are rejected. Initialization must create new custody and refuses
existing files; reopening must find the exact existing custody and never silently
recreates a missing journal. A partial publication is retained for diagnosis,
not repaired by creating a new original goal.

## Complete tool workflow

1. Open with `fortress.open_session(mode="control", max_wall_millis=60000)`.
   Default mode is `offline`; initial creation requires the explicit operator designation gate and checkpoint
   exception. Preserve the returned journal identity even before a plan exists.
2. Call `fortress.observe(session_id)`. Keep its exact observation seal. Optional
   `fortress.query` pagination accepts only a JSON string with `mode="tiles"`,
   `expected_observation`, `offset` and `limit` (1..32). Pages refer to the same
   retained observation and do not acquire new game state.
3. Pass the following inner JSON as the `request` string to `fortress.plan`.
   Substitute the exact observation seal and an absolute future deadline:

```json
{
  "expected_observation": "EXACT_64_CHARACTER_LOWERCASE_OBSERVATION_SEAL",
  "kind": "mine",
  "target": [4, 4, 5, 2, 2],
  "deadline_tick": 200,
  "poll_interval_ticks": 10,
  "stable_samples": 2
}
```

The target means `[x,y,z,width,height]`. Deadline must be later than the original
observation and at most one game year away. The requested cadence and 1..64 stable
samples must fit before that deadline. Omitted cadence defaults to 10 ticks and
omitted stability to one sample. Unknown or duplicate fields, raw native keys,
hidden-neighbor overrides and tool-supplied source/checkpoint policy are rejected.

4. Review the complete original goal and paired original/native seal. Preparation
   records no mining designation. Call `fortress.commit(session_id, paired_seal,
   confirmed=true)` once. The coordinator checks current authority, exact original
   custody and fresh independent terrain after durable intent and again at the
   native dispatch boundary. Human review may span foreground requests. Each
   newer trusted request in the same session receives a wall window bounded by
   the negotiated allowance and 60-second cap; callbacks within that request
   can only narrow its deadline. The original connection, preparation permit,
   prepaid byte allowance and game-time limits remain unchanged. An active
   timeout, source error or authority failure permanently fences the connection;
   a later request cannot revive it or recover a commit permit by reconnecting.
   A lost reply is reconciled using the original key.
5. Call `fortress.wait(session_id)` for original-key recovery or bounded goal
   monitoring. After `Designated`, the first wait initializes `goal.history` and
   returns without a current terrain sample; the next wait acquires the first
   sample. A designated wall remains incomplete; a dry, cleared floor may satisfy
   the original goal only with its required stable samples. Keep the returned
   exact recovery object whenever its history-presence flag changes.
6. Use `fortress.query`, `fortress.explain` or `fortress.doctor` to inspect retained
   local custody. `fortress.cancel(scope="goal")` retires a still-prepared native
   action; it does not clear designation flags or undo excavation.
   `fortress.cancel(scope="session")` releases sockets and file locks while
   retaining durable custody, even after revocation or fencing.

**This runtime does not advance game time.** Mining needs game time to progress
under external control. Wait performs no unpause, background polling or automatic
run chaining. Multiple observations at the same tick cannot manufacture temporal
stability. There is no `kind="run"` action in this profile. `fortress.checkpoint`
and `fortress.restore` return explicit capability denials.

## Exact restart and uncertainty

For reopening, set `initialize=false`, supply the exact top-level `journal_id`,
and retain the original source configuration and directory. Once an original
archive exists, also copy the complete returned `recovery` object into operator
configuration. Its exact plan/object digests, original anchor and `history_present`
flag are required. Optional publication-diagnostic booleans can remain present.
Do not replace an original anchor with the most recent observation anchor.

Open in `offline` for local evidence only, or `reconcile` for independently
qualified native/spatial reads. Recovery cannot reconstitute the original live
commit permit, even if a queried native record is still `Prepared`. Explicit
retirement is available only with current operator designation authority. A
permanent native `Unknown` remains indeterminate; repeated queries or a fresh key
cannot establish whether that original dispatch happened. Preserve its handoff
and release local custody when no further safe action is available.

Replay requires freshly supplied historical evidence policy. Retained fingerprints
only compare policy; they never restore grants. Current goal truth is cleared
on reopening and unfinished stability is interrupted. Historical first achievement
remains separately visible. Source-incarnation changes, regressed clocks or
corrupt/replaced custody fence the owner instead of renewing its deadline or key.

A failed foreground wait immediately withdraws its cached current goal, including
failures before a native read or while publishing the response. The next permitted
wait records that known gap within its own budget before another sample can
continue unfinished stability. A later query cannot restore the withdrawn proof;
historical first achievement remains visible. Recovery references also omit
cached proof and preparation/enablement flags while preserving exact identities.

The default logical byte-work allowance is 2 GiB; callers may explicitly request
up to 64 GiB for larger bounded source work or historical replay. These account
for repeated verification
work and do not allocate that amount of memory. Complete known planning work is
prepaid before original archive publication, so an insufficient allowance does
not strand an unprepared original plan. Wall time defaults to the 60-second cap
and may be explicitly narrowed;
response reservation is 128 KiB, and goal history is bounded to 64 MiB and 1024
events without silent pruning. Errors and oversized responses retain bounded exact
recovery identities while withdrawing cached goal and control claims.

This slice does not implement multi-region planning, generic action DAG execution,
clock advancement, checkpoint/restore, mining reachability, structural-safety
certification or exclusion of other game controllers. These remain separate work.
