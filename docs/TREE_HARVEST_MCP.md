# One visible tree development supply path

Tree-harvest/1.21 belongs to original `dfmcp-reality-wiring-proof-4bf.14`.
It is an isolated, unadmitted development profile. It adds one supported
`DFHack::Designations::markPlant` designation, followed by foreground observation
of a root-coordinate predicate and eligible ground WOOD. It does not establish
live felling, tool acquisition, scheduling, a workshop, a bed or admission.

Fresh-embark scouts found the three logs were reciprocal PERM materials of the
Wagon. Imported flags and operator permission do not make structural materials
free. This route neither borrows them nor changes workshop/1.20's ground-only
policy. It does not detach items, change flags or force jobs/equipment.

`dfmcp-tree-harvest-v1-21-dev-server` preserves the existing eleven modern MCP
tool names. Directly callable names use underscores; dotted operation tags in
Agent Turns remain semantic labels. No client argument chooses a root, protocol,
source, native address, path, endpoint, arbitrary command or Lua.

## Scope and evidence

The host pins one visible mature-root coordinate, world/site, software versions,
generation and map dimensions. There is no stable plant ID in this SDK. Root and
species continuity are coordinate predicates, never original entity identity.
Capture validates bounded tree dimensions and every cell of the body cuboid plus
a horizontal halo for visibility before serializing body data. Belowground roots
remain unobserved. The exact supported southeast-trunk designation coordinate is
computed by `getPlantDesignationTile`, and may differ from the root. Worker and
tool approach facts refer to that computed target.

The complete loaded-map read scope is explicit because this slice scans bounded
plant, job, citizen and visible-tool/item registries. Hidden-domain absence is
not inferred. Direct designation authority and the process-local exclusive lease
cover the observed tree/body/halo plus the entire touched 16x16 scheduling block.
The host effect ceiling must contain that derived cuboid, and protected regions
must not intersect it. Exact scoped grants are supported; a global mutation
grant is not required. The local lease does not globally fence controllers.

An eligible CUTWOOD worker is living, active, adult, sane, visible, a citizen,
labor-enabled, unit-specifically labor-valid, job-available and observed connected
to a target approach. An axe offer has bounded exact unclaimed flags/references
and a reciprocal TEMP Wagon relation. This is an observed offer, not equipment,
reservation, acquisition, assignment or a scheduling guarantee. PERM roles fail.

`markPlant` changes the normal Default chop designation and block designated bit.
The insertion receipt independently double-captures that exact poststate. The
internal shared engine's phase `Placed` means designation acknowledged here;
it never means a felled tree or produced wood. Ordinary mine generations 1.15
and 1.16 continue to exclude TREE.

## Closed host launch

The only accepted DFMCP environment names are:

```text
DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21=1
DFMCP_TREE_HARVEST_MCP_CONFIG=<closed JSON>
DFMCP_TREE_HARVEST_TOKEN=<operator-held 32..256-byte native credential>
DFMCP_TREE_HARVEST_ALLOW_DESIGNATE=1
```

Any present production launch/protocol state or another DFMCP profile refuses
before stdio starts, with nonzero process status. The native plugin has its own
1.21 opt-in, credential and write gate. The dedicated process uses the existing
process-scoped family 31; this number is also used by other isolated development
profiles, so process isolation and closed launch state are required. No core ID
format is enlarged and no production ticket is consumed.

The configuration is environment JSON, not an externally mutable config file:

```json
{
  "schema": "dfmcp.tree-harvest-mcp-development/1",
  "directory": "/private/tree-harvest",
  "endpoint": "127.0.0.1:5000",
  "initialize": true,
  "acknowledged": null,
  "world_folder": "<accepted source folder>",
  "site_id": 0,
  "df_version": "<observed exact version>",
  "dfhack_version": "<observed exact version>",
  "generation": 1,
  "recovery_generation": null,
  "map_dimensions": [64, 64, 8],
  "root": [15, 15, 2],
  "read_scope": [0, 0, 0, 63, 63, 7],
  "scope": [0, 0, 0, 63, 63, 7],
  "protected": [],
  "authority_expires_tick": 20000,
  "lease_ticks": 1200,
  "checkpoint": "disposable-fortress-no-checkpoint"
}
```

Example numbers grant nothing; replace them with independently accepted facts.
The existing absolute owner-private 0700 directory must already exist. New
custody exclusively creates regular single-link 0600 `tree-harvest.journal` and
`tree-harvest.ack`; missing, corrupt, empty or rolled-back recovery files refuse.
There is no checkpoint provider or automatic rollback.

## One original attempt and foreground proof

Open control, observe the fixed offer, and call `fortress_plan` with a JSON
`request` containing only `kind="fell_one_visible_tree"`, exact
`expected_observation`, and `deadline_tick`. Review root/species/body, computed
target, source, worker/tool offers, effects, lease and the exact review seal.
Preparation does not change the game. The original horizon is at most 12000
ticks and cannot be renewed. A monitoring horizon advances no clock.

Confirm exactly that seal with `fortress_commit(..., confirmed=true)` on the
original prepared connection. The journal synchronizes Dispatched, then the
independently retained acknowledgement, then rechecks authority/lease/guard
before one native dispatch. Dispatch consumes local permission before any
fallible entry check. Failed budgets, cancellation, revoked grants or transport
errors cannot revive it when a valid context returns. Query-cleared and other
connections cannot borrow preparation. Each operation shares one shrinking wall
deadline and charged byte allowance across I/O, guard, ACK and writer.

`fortress_wait` reconciles the same native key or takes one fresh progress sample.
The journal acknowledges a sample-start latch before reading; lost publication
cannot silently become a new goal. Full-history generation/tick/sequence/item
high-water and sticky original-continuity loss survive reopen. Indeterminate
native history can become only indeterminate or verified designation; negative
receipts cannot erase uncertainty. Post-writer custody failure reports the exact
original key as indeterminate and never grants another writer.

The foreground goal requires a paused fresh same-source/epoch observation:
the root mature-tree predicate is absent, no nearby felling job is observed, and
new eligible ground WOOD exists above the original item-ID horizon. Item rows
are sorted by ID. Spatial coincidence does not establish causal item provenance.
The workshop/1.20 native read must independently qualify a selected fresh log
and its exact nine-floor candidate before constructing the original workshop.
Repeated Wait retains historical achievement without relabeling it fresh.

Keep both custody files and the externally acknowledged `{journal_id,head,frames}`
pin. Recover and offline modes use `initialize=false`; checked descendants may
resume evidence but never restore dispatch permission. Offline performs no
native calls or custody writes. A valid-prefix journal rollback is rejected by
the independent ACK even after a lost MCP reply. This is local crash custody,
not protection against rollback of both stores and every external pin.

Every error and overflow keeps the canonical Agent Turn spine. Canonical world
anchor is null and continuity explicitly unestablished. Native references and
verified local custody are separate. A pre-effect invalid request has no invented
indeterminate effect or null reference. Actual dispatch uncertainty retains its
original key, seal and no-retry requirement.

## Clock and qualification boundaries

Broader world evolution and future falling-tree consequences require separately
authorized clock exposure; visible body checks do not certify falling safety.
The existing standalone run/1.13 profile remains limited to separately confirmed
1200 ticks and 60000 wall milliseconds per run. Multiple reviewed runs may fit
the fixed monitoring horizon; no run extends that horizon, grants tree authority
or proves causality. No new clock implementation is added.

Both mandatory full commands retain all 43 gate IDs and add Tree checks inside
existing gates. The machine contract and static checker bind exact source/test
names, original vectors and native mutants. Cargo registers the actual five-case
modern stdio subprocess suite in debug and release. Native component tests use
SDK/protobuf doubles, not DF. Matching real-SDK builds of both Tree and workshop
are required because they share the extracted one-attempt engine. Source/focused
execution, full Rust qualification, native R1, live proof, cleanup and admission
remain distinct. Original `.14` workshop and finite-bed criteria stay open until
their independent real acceptance evidence exists.
