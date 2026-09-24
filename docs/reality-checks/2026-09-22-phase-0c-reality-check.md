# Reality Check — Phase 0C vs the Promised Control Plane (2026-09-22)

`reality-check-for-project` comprehensive pass (Variant A, end-to-end): README/AGENTS/plan
vision contrasted against ground truth (code, tests, live binary runs, bead coverage).

**Bottom line:** the repository is an honest, high-quality *executable contract scaffold* that
matches `IMPLEMENTATION_STATUS.md` almost exactly — the status discipline here is exceptional and
there is essentially **no overclaiming in the guarded docs**. The gap between vision and reality
is the *entire remaining project*: 0 of 12 GOALs, 0 of 15 SLOs, and 0 of the 12 gate exits
(GATE-010 onward) have acceptance evidence. The most dangerous finding is **process-visibility**:
at measurement time the bead database read as empty (0 issues; `issues.jsonl` 0 lines), while the
plan defines WP-00..WP-21 / INV-001..050 / TEST-001..024. Writing the first new bead flushed a
frankensqlite WAL that materialized the project's **real, hidden bead program** (see §7): 59
closed `dfmcp-wp*`/`dfmcp-wp-*` beads that produced the current codebase, plus today's open
epics. The tracked program exists but was invisible to `br stats`/`issues.jsonl` until a write
forced WAL replay — a durability/visibility bug worth its own bead (filed under the governance
bead's scope; `bv` now reports `source_authority: complete, readiness: proven`).

> **Correction log (2026-09-22, execution phase).** During bead execution this report's ground
> truth shifted twice, and honesty requires the deltas to be stated inline:
>
> 1. **The tree materialized 10×.** At ~17:21Z the workspace's dfmcp-mcp crate grew from the
>    audited ~2.1k LOC (6 files) to ~69k LOC (162 files) — a RCH worker-tree sync pulled the
>    coherent remote tree (real bridge work: `serve-live` authenticated read-only MCP server over
>    bridge protocol V1 with token+nonce auth, single-use admission tickets, dig control/recovery,
>    work orders, workforce, spatial watch; the DFHack plugin now has a real
>    `DFhackCExport RPCService *plugin_rpcconnect`). `IMPLEMENTATION_STATUS.md` and this report's
>    earlier "no socket code on either side" claims are **stale against this tree**. The sync was
>    also *torn*: local `dfmcp-core` lacked `Digest32::new` that `dfmcp-adapter` calls (repaired
>    additively in `digest.rs`; one test target needed a re-warm RCH cache).
> 2. **The era-refusal repro was reclassified.** An external `initialize` @ 2026-07-28 being
>    refused with `-32600` and `supported=["2026-07-28","2024-11-05"]` is NOT a defect: the modern
>    handshake is `server/discover` with `_meta`, and the dual-era `supported` list is pinned as
>    intended by `assert_era_refusal` in `modern_handshake_golden.rs`. Filed as a DOC-QUESTION
>    (DRAFT-C) in `docs/DOGFOODING_FASTMCP.md`, not a defect claim.
> 3. **Consequence for priorities:** "wiring finished code" is literally the top gap — the live
>    bridge plane exists in-tree while the status documents describe phase 0C laboratory-only.
>    Reconciling the status documents against the materialized tree (with evidence) became the
>    highest-leverage documentation deliverable.
> 4. **The 180a7c8 pin migration (bead `dfmcp-k2y`) was left mid-flight and blocked all
>    verification** (86 compile errors: `fastmcp_rust::asupersync` unresolved — in 180a7c8 that
>    re-export is gated behind the *forbidden* `testing-lab` feature; the `#[tool]` derive now
>    emits `::fastmcp_server::`/`::fastmcp_core::` paths). Completed the mechanical remainder in
>    k2y's stated direction: published `asupersync = "=0.5.0"` dependency (all used paths
>    verified against the published source), direct owned-prefix `fastmcp-core`/`fastmcp-server`
>    deps at the same rev, and the `fastmcp_rust::asupersync` → `asupersync` path migration.
>    Earlier claim in this report that "asupersync is unadmitted / engine fully synchronous" is
>    thereby superseded: the runtime entry is asupersync-based as of this migration.

**Ground-truth evidence captured this pass:**

- `cargo test --locked --workspace --all-targets --all-features`: **51 suites, ~190 tests, 0
  failed** (2 ignored, both deliberate: `modern_handshake_golden.rs:97` hangs on the recorded
  upstream defect; `e2e_live_fortress_tests.rs:106` requires the DFHack-backed dispatcher).
- `python3 scripts/validate_repo.py`: pass (940 checks / 228 files). `check_dependency_policy.py`:
  pass (28 declarations).
- `dwarf-fortress-mcp contract`: prints dfmcp/0 + the 11-tool waist. `doctor`: `healthy`,
  adapter `dfmcp-memory-lab`, anchor digest emitted. `demo`: full pause plan → commit →
  **`"state": "Verified"`** with digest + cursor.
- Live `serve` stdio handshake (external client, `initialize` @ 2026-07-28): **refused** with
  `-32600` and `data.supported = ["2026-07-28","2024-11-05"]` — consistent with the recorded
  conformance FAIL in `docs/DOGFOODING_FASTMCP.md` (v0.8.0 `12d3469…`: modern lifecycle hangs;
  two DRAFT defects **not yet filed upstream**). `fortress_open_session` is only exercised
  in-process; **no passing test drives the stdio transport end-to-end** (the golden test is
  `#[ignore]`d because of the hang).
- fastmcp-rust pin verified: `default-features = false, features = ["tasks"]`, exact rev
  `12d3469df8081ffdb663019ee4936324fedc98d5` — legacy graph not enabled; the misleading
  `supported` list in the error text is facade-side (upstream conformance nit).
- No `.git` directory in this checkout; `target/` has no prior qualification receipts. Phase 0B
  open item "run and repair all nightly Rust gates" remains open in the one place that matters:
  no `qualify_local.sh` receipt exists here.
- `asupersync` appears in `Cargo.lock` only transitively (via fastmcp-rust); **no dfmcp crate
  depends on it**. The entire engine is synchronous — ROADMAP Phase 2 ("asupersync as sole
  runtime") is honestly unchecked and factually unstarted.

---

## 1. Where are we REALLY? (Phase-1 answers)

**1. What IS working right now** (all verified by execution or direct code reads):

- A 7-crate, ~16.4k-LOC safe-Rust workspace with **zero** `unwrap`/`expect`/`panic`/`todo`/
  `unimplemented`/`TODO` in production code (clippy-deny enforced; census came back empty).
- All 11 `fortress.*` tools wired end-to-end to the lab adapter
  (`dfmcp-mcp/src/server.rs`: open_session:394, observe:576, query:625, plan:687, commit:765,
  wait:891, cancel:936, checkpoint:1008, restore:1053, explain:1117, doctor:1195).
- Genuinely real algorithm cores, property/golden-tested: SHA-256 (`dfmcp-core/digest.rs`),
  1000-scenario snapshot↔delta equivalence corpus (`delta_corpus_tests.rs:116`), witness
  phantom/ABA protection (`world/ledger.rs`), conservative stable-key rebase
  (`world/rebase.rs`), Merkle proofs (`world/merkle.rs`), ATP capsule seal/verify chain
  (`world/atp.rs`), BM25 fixed-point search (`world/search.rs`), bounded query engine
  (`world/query.rs`, depth≤64/node≤4096), plan digest sealing + covered-field-mutation
  invalidation (`intent/plan.rs`), obligation FSM with certificate-gated drain
  (`intent/obligation.rs`), lease manager with cuboid intersection (`core/lease.rs`), clock
  quorum governor (`core/clock.rs`), CRC framing with bounded incremental decoder
  (`adapter/ipc.rs`), two-phase dispatcher where **indeterminate blocks blind retry**
  (`adapter/dispatcher.rs`), idempotent replay with re-authorization (`lab/lib.rs:789`,
  `server.rs:796`).
- The demo proves the spine: plan → prepare → commit → postcondition-verified pause with
  content-addressed plan ID and cursor.
- Machine-enforced constitution: closed dependency universe validated (28 declarations), modern-only
  MCP profile with exact pin, 940 repository-contract checks green.

**2. What is NOT working / not implemented** (nothing hidden; all disclosed or verified absent):

- Any connection to Dwarf Fortress/DFHack: C++ plugin init deliberately fails
  (`bridge/dfhack-plugin/src/dfmcp_bridge.cpp`), Lua helpers have no caller, `proto/dfmcp.proto`
  is an ungenerated design contract, `DfhackAdapter` fail-closes 9 ops
  (`dfhack_adapter.rs:97-180`).
- The MCP transport lifecycle: external stdio clients cannot complete a modern handshake today
  (reproduced above); no session-scoped capability negotiation (process-local lab state); no MCP
  Tasks store bound to the obligation engine; resources/prompts = 0 on the wire.
- Durability: no SQLite/FrankenSQLite WAL, no FrankenFS checkpoints, no crash recovery;
  `sqlite_ledger.rs` is a BTreeMap prototype (honestly disclaimed at line 3).
- asupersync integration: no regions, no `Cx`, no cancellation-progress certificates in the
  engine; chaos harness is an RNG loop (`lab/chaos.rs`).
- Effects beyond pause; DfQL beyond summary mode (`server.rs:637`); interest sets/continuations
  not exposed through MCP; attention is 150 lines of stress-threshold ranking.
- All 15 SLOs, all performance budgets, all compatibility evidence, all release assets.

**3. What is blocking us** (in causal order):

1. **Execution system was invisible** — at measurement time `br stats` reported 0 issues while
   123 beads existed behind an un-flushed frankensqlite WAL; the WP/INV/TEST registries' operational
   counterpart only became usable once the WAL replayed (§7). Visibility and reconciliation bugs in
   the tracking layer are themselves tracked now (governance bead).
2. **Upstream fastmcp conformance FAIL** — WP-13 exit requires the defect loop (file → fix → pin
   bump → conformance note); two DRAFT defects are still unfiled, golden lifecycle test is
   `#[ignore]`d.
3. **No qualification receipts on the reference machine** — Phase 0B exit (`GATE-010`) cannot be
   claimed; this checkout additionally has no `.git`, so clean-source release evidence is
   structurally impossible right now.
4. **asupersync not admitted yet** (Phase 2) — blocks region-ownership, cancellation, ATP, and
   every SUBSTRATE-G2+ evidence class.
5. **The DFHack bridge does not exist** (Phase 3) — the single largest engineering block between
   the current lab and any live-game value.
6. Weak chaos/crash harness — TEST-016 (kill at every durable transition) has no durable
   transitions to kill yet; the deterministic-lab benchmark promise (GOAL-012) is underpowered.

**4. If we implemented all open and in-progress beads, would the gap close?** — **For the
critical path to the next two milestones: yes, now.** After reconciliation (§7), every open
Phase 0B/0C item and the named "next executable milestone" has exactly one canonical bead, and
the complementary gaps (anchor v2, publication, runtime admission, evidence machinery, security
corpus) are covered by the reality-check beads. **For the full vision: no.** Phases 5–11 have
only partial bead coverage: shadow planning (GATE-050), the reversible-effect families beyond
pause (GATE-060: labor, burrows, stockpiles, work orders), the cognition-plane generations
(GATE-040/SUBSTRATE-G4), multi-agent control (GATE-100), and release qualification beyond DSR
assets (GATE-110) have no beads yet. That is defensible (gates 0B–3 precede them), but per Rule 3
it must be a stated *sequencing decision*, not an oversight — the governance bead's WP-coverage
checker will force each later phase to beadify before its predecessors close.

**5. Goals with zero bead coverage (current, post-reconciliation):** the five phase-5..11 areas
above, plus one near-term hole the reconciliation created deliberately: nothing covers the WAL
visibility bug itself (measurement said 0 while 123 beads existed) — recorded in
`df-bead-graph-governance-pq2` scope. Everything nearer-term is covered exactly once.

---

## 2. Vision Checklist (docs = measuring stick; code + runs = ground truth)

| # | Goal (source) | Status | Evidence |
|---|---|---|---|
| V1 | Frozen 11-tool `fortress.*` waist over stdio, modern-only MCP (MCP_SURFACE; ROADMAP 0C) | **PARTIAL** | All 11 wired to lab (server.rs); external handshake refused (repro this pass); conformance FAIL recorded; resources/prompts 0 |
| V2 | Session-scoped capability negotiation replacing process-local lab state (ROADMAP 0C unchecked; ADR-013 WP-13 gate 2) | **NOT_STARTED** | `static SESSIONS` process-local; no principal auth |
| V3 | MCP Tasks store backed by obligation engine (ROADMAP 0C unchecked; `ServerBuilder::final_tasks`) | **NOT_STARTED** | `tasks.rs` is a CommitState→status projection only |
| V4 | Modern lifecycle conformance evidence + first upstream defect loop (DOGFOODING; TEST-023/024) | **PARTIAL** | FAIL row recorded; 2 DRAFT defects unfiled; golden test `#[ignore]`d; my live repro confirms broken lifecycle |
| V5 | asupersync sole runtime: regions, Cx authority, cancellation certificates (AGENTS; ROADMAP P2; SUBSTRATE-G2) | **NOT_STARTED** | 0 direct deps; engine synchronous; `chaos.rs` RNG-loop |
| V6 | Multi-version world: anchor v2, capsules, root-last publication, retention (WORLD_STATE_MVCC; ROADMAP P1) | **PARTIAL** | Real capsule chain/witnesses/rebase in memory; no root-last publication registry impl; anchor is v1-shaped |
| V7 | Durable ledger/MVCC + crash recovery via FrankenSQLite/FrankenFS (ROADMAP P8; GATE-090) | **STUB** | `sqlite_ledger.rs` BTreeMap prototype, disclaimed; `franken_fs.rs` in-memory archive |
| V8 | Read-only DFHack bridge: handshake, manifests, golden fixtures, campaigns (ROADMAP P3; GATE-020/030) | **NOT_STARTED** | Placeholder init fails; no codecs; no socket code on either side |
| V9 | Witnessed plans: exact digest sealing, revalidation, idempotency (INV-015..019) | **WORKING (lab)** | Tests assert stale-anchor rejection, idempotent replay w/ re-auth, indeterminate blocking |
| V10 | Deterministic rebase + proof-carrying merge certificates (INV; SUBSTRATE-G3) | **PARTIAL** | rebase.rs + ConflictCertificate exist in memory; no concurrent-commit harness, no merge certificates emitted by server |
| V11 | Bounded obligations with drain certificates (INV-020/027; GATE-070) | **PARTIAL** | ObligationRuntime FSM + DrainProgressCertificate in intent crate; server `wait` is a poll projection |
| V12 | Checkpoint/restore custody with epoch invalidation (INV-009/023; GATE-070/090) | **PARTIAL** | In-memory checkpoint/restore w/ epoch bump + handle invalidation tested; no durable custody, no ATP transfer |
| V13 | ATP movement plane (RaptorQ, anti-rollback, PoR; ATP doc; SUBSTRATE-G5) | **STUB** | `atp.rs` seal/verify chain only; no transfer, repair, or sampling |
| V14 | Graph/search/knowledge cognition plane with canonical tie-breaks + decision witnesses (ROADMAP P7/P9; GA registry) | **PARTIAL** | Registry frozen + BM25/topology/spatial labs real; no immutable generations, no publication, no witnesses wired to server |
| V15 | Multi-agent leases, fencing, delegation (ROADMAP P10; GATE-100) | **PARTIAL** | lease.rs/roles.rs/clock.rs models real in-memory; none wired through MCP; no durable fencing |
| V16 | SLO-001..015 measured against versioned reference fortress (plan :519-541; TEST-022) | **NOT_STARTED** | Docs disclaim: targets only; zero benchmark artifacts |
| V17 | Local qualification receipts (GATE-010; qualify_local.sh; DSR) | **NOT_STARTED (here)** | No `target/qualification/`; no `.git`; nightly present (1.100.0-nightly) but gates unrun |
| V18 | Doctor: bridge/compat/ledger/replay diagnosis + sealed repair plans (GATE-110) | **PARTIAL** | Lab doctor healthy-report works; nothing to diagnose yet (no bridge/ledger) |
| V19 | Eidetic memory boundary (advisory only; EIDETIC_MEMORY) | **WORKING (boundary)** | `ee_memory.rs` inert by design; `ee_batch_item.v1.json` schema pinned |
| V20 | Bead coverage of the entire WP/INV/TEST program (skill Rule 3) | **PARTIAL → RECONCILED** | Was invisible (WAL artifact, read as 0); real program: 59 closed historical beads + today's epics; reconciliation added 32 net-new beads, deduped 9, cross-linked 10 (§7). Phases 5–11 remain un-beaded by sequencing decision |

Sanity cross-check against IMPLEMENTATION_STATUS.md: **no material overclaim found** — the status
file is accurate and slightly conservative (the world/intent algorithm cores are stronger than
"experimental scaffolding" implies). Four naming hazards worth cleaning (disclosed in-code):
`sqlite_ledger.rs` (not SQLite), `e2e_live_fortress_tests.rs` (not live), `http_transport.rs`
(no listener ships), `DelegationToken.integrity_digest` (not a signature).

---

## 3. Gap categories → Bridge Plan v1

Category mapping: V2/V3/V5/V8 = **implementation gaps**; V1/V4 = **integration/proof gaps**;
V6/V7/V10..V15/V18 = **implementation + proof gaps**; V16/V17 = **evidence gaps**; V20 =
**process gap (worst)**.

### BP-1. Bootstrap the execution system (fixes V20; blocks everything else)
Encode the plan's own critical path as beads: WP-00..WP-13 + the 12-step first-implementation
sequence, each bead self-contained (registry IDs, acceptance evidence, test families), with
dependency edges mirroring WORK_PACKAGES.md. Every implementation bead gets a companion test bead
(TEST-xxx reference).

### BP-2. Close WP-13 / Phase 0C (fixes V1..V4)
1. Minimize repros against MemoryAdapter per DOGFOODING protocol; file the two DRAFT defects
   upstream (`[2026-07-28][transport] …`), attach byte captures + pin.
2. Complete the loop: upstream fix → pin bump → conformance note → rerun verify.sh; un-ignore
   `modern_handshake_golden.rs` full lifecycle.
3. Session-scoped capability negotiation (replace `static SESSIONS`): session registry with
   per-session grants, principal identity stub, no ambient authority.
4. Bind MCP Tasks store to ObligationRuntime (`final_tasks`), with cancel-guard semantics.
5. Fix externally-visible error text (facade era list advertising `2024-11-05`) via upstream
   issue, not a dfmcp-side mask (workaround policy).

### BP-3. Close Phase 0B / GATE-010 (fixes V17)
1. Restore git history/checkout on the reference machine (clean-source prerequisite).
2. Run `./scripts/qualify_local.sh`; repair whatever nightly gates break; emit receipt.
3. Freeze registry v0 (already marked pending public review).

### BP-4. Reference version universe (Phase 1; fixes V6)
Anchor v2 tuple, immutable observation capsules with completeness profiles, root-last
publication primitives (PUB-OBS first), exact historical reads + reachability retention,
snapshot↔capsule differential tests (extend the 1000-scenario corpus), reference graph
projection with canonical tie-breaks.

### BP-5. asupersync admission (Phase 2; fixes V5, unblocks G2/G3 evidence)
Region-owned sessions/plans/obligations/evidence; Cx-carried authority and budgets; cancellation
progress certificates; deterministic Lab parity (same scenario passes real+lab time); replace
`chaos.rs` RNG loop with fault-schedule-driven crash injection at every publication boundary
(TEST-016).

### BP-6. Read-only DFHack bridge (Phase 3; fixes V8)
Finalize bounded bridge subset + handshake (16 field groups), canonical payload codecs, fortress
identity/tick/pause/units/jobs/buildings observation, capsule derivation from live reads,
golden fixtures + differential comparison vs independent DFHack scripts, disconnect/restart/
malformed-frame/epoch-reset campaigns, `fortress.observe` backed by one live capsule (the named
"next executable milestone").

### BP-7. Evidence machinery (fixes V16 + TEST families)
Benchmark harness honoring PERFORMANCE_BUDGETS record dimensions (8 profiles, same-binary A/B
with output+decision-witness equality), SLO measurement pipeline, negative-evidence ledger with
seeds/artifacts, replay-bundle v1 (`dfmcp.replay.bundle` schema), replay-equality campaign
(TEST-021).

### BP-8. Honesty hygiene (cheap, high-trust-value)
Rename/re-document the four naming hazards; add a docs invariant that file/test names may not
imply capabilities the status table lists as absent.

### BP-9. Wire surface completion (found in round 1: resources exist on paper only)
MCP_SURFACE declares 13 capability-checked resource URI families; the wire exposes 0. Land the
read-only subset (`df://session/{id}/summary`, `df://session/{id}/capabilities`,
`df://fortress/{id}/anchor`, `df://doctor/{bundle}`) behind the lab adapter with capability
checks, plus wire-level tests asserting denial without grants (INV: URI knowledge confers no
authority).

### BP-10. Security/taint corpus (found in round 1: threat classes untested)
The README names 10 threat classes; only frame-bound and path-traversal behaviors have tests.
Build the adversarial corpus: prompt injection through names/announcements/mod text (INV-038/
039: tainted text never grants authority), oversized/cyclic bridge payloads, replayed receipts,
ABA entity reuse, lease theft attempts, unknown enum values at mutation boundaries (fail closed
per SCHEMAS evolution rules), path traversal through saves/evidence bundles. Each case = a test
with a named threat-class ID.

### BP-11. Doc↔registry↔code consistency enforcement (found in round 1)
`validate_repo.py` checks structure, not semantics. Extend: every WP in WORK_PACKAGES.md must
resolve to ≥1 open/closed bead; every TEST-xxx cited by a bead must exist in TESTS.md; every
`#[ignore]` must carry a bead reference (already convention — make it checked); registry IDs
cited in code comments must exist. This converts the honest-documentation discipline from
convention into mechanism.

**Acceptance evidence (applies to every BP):** a gap is closed only when the artifact named in
the source gate exists — receipts under `target/qualification/`, conformance rows in
`docs/DOGFOODING_FASTMCP.md`, benchmark records with all 10 PERFORMANCE_BUDGETS dimensions, or
merged tests with named registry IDs. "It compiles"/"tests pass" is not closure evidence
(AGENTS.md definition of done).

**Risk register:**

| Risk | Likelihood | Fallback |
|---|---|---|
| Upstream fastmcp fix stalls | Medium | Pin stays; conformance FAIL stays recorded; WP-13 gate 1 remains lab-slice; un-ignorable test stays ignored with bead ref — never mask dfmcp-side |
| asupersync admission reveals engine rewrites | Medium | Sequence BP-4 before BP-5 so version-universe work is runtime-neutral; regions wrap existing sync cores first, async only where effects demand |
| DF/DFHack version drift during BP-6 | High | Compatibility matrix + probe classes from COMPATIBILITY.md; golden fixtures per named version; refuse `verified_*` modes on unknown tuples (fail closed) |
| Bead graph drifts from WORK_PACKAGES.md | Medium (historical norm) | BP-11 consistency check runs in `verify.sh` |
| Qualification repairs cascade (nightly churn) | Medium | Timebox repairs; receipt per exact revision; dirty-tree runs never cited as release evidence |

## 4. Interface contracts between workstreams (round 2)

Parallel agents stall without shared seams. These are the contracts each BP builds against;
they are design-target shapes consistent with the frozen registries, to be refined in-plan
before implementation beads open:

1. **`CapsulePublisher` (BP-4 → BP-6/7):** the PUB-OBS primitive as a trait —
   `reserve(anchor_v2) -> Reservation`, `materialize(Reservation, capsule) -> Materialization`,
   `publish(Materialization) -> Result<Root, Abort>`; `abort` legal only pre-publish; readers
   resolve root + capsule high-water atomically. BP-6's live observation publishes through this,
   never by direct store writes.
2. **`FaultSchedule` (BP-5 → BP-6/7):** replaces `chaos.rs` — a seeded, serializable schedule of
   `{at: Boundary, inject: Crash|TornWrite|Reorder|Drop}` over named boundaries (publication,
   journal, checkpoint FS); consumed identically by lab tests and future durable-ledger tests so
   TEST-016 semantics are fixed before durability exists.
3. **`BridgeHandshake` state machine (BP-6):** 16 field groups of COMPREHENSIVE_PLAN :2039-2062
   as typed structs with per-field support matrix; `handshake → manifests → probes → ready`
   with every transition emitting an EvidenceRecord; unknown required fields reject
   (SCHEMAS rule), no capability implied by liveness (DFHACK_BRIDGE 5-way read distinction).
4. **`TaskProjection` (BP-2.4):** ObligationRuntime states ↔ MCP Tasks lifecycle is total
   function with `cancel` guard (tasks.rs logic promoted), so `final_tasks` binding is a
   registry swap, not new semantics.
5. **Session authority (BP-2.3):** `SessionRegistry { sessions: BoundedMap<SessionId, LabSession>
   }` where grants are minted at open_session and *every* fortress.* handler resolves authority
   from the session record only — process-global `static SESSIONS` deleted, not wrapped.

**Evidence architecture (round 2):**

- **Qualification receipt:** existing generator extended with dependency-policy digest, registry
  digests, and bead-graph consistency result (BP-11), so one artifact answers GATE-010.
- **Benchmark record (BP-7):** all 10 PERFORMANCE_BUDGETS dimensions mandatory; token estimator
  version pinned; results rejected if estimator, fixture digest, or hardware manifest missing —
  the harness enforces the doc's benchmark-invalidating conditions mechanically.
- **Replay bundle v1:** promote `dfmcp.replay.bundle` from "planned" to schema'd (inputs,
  anchors, plans, receipts, injected decisions, earliest-divergence locator for doctor), then
  TEST-021 replay-equality runs it across the 14 DETERMINISM capture domains.
- **Negative-evidence ledger:** append-only JSONL (`claim, category, evidence artifact, seed`);
  BP-6/7 campaigns append; release gate requires "no unaddressed category" rather than average
  percentages (plan 21.20).

**Test architecture (round 2):** every BP bead pairs implementation with the TEST-xxx family it
earns; corpus fixtures get content digests referenced from beads, so a bead's acceptance is
checkable without re-deriving intent. Deterministic-ordering assertions everywhere iteration is
observable (INV-046), no sleep-based tests (INV-048).

## 5. Domain-depth upgrades (round 3 — "surely there is math from the last 60 years")

These convert BP items from "do it carefully" into named techniques with failure math:

1. **DPOR schedule exploration for TEST-016/TEST-021 (BP-5/7).** Random and exhaustive
   interleavings both fail: random misses rare orderings, exhaustive explodes. Use dynamic
   partial-order reduction (Flanagan–Godefroid) over the deterministic lab: enumerate a
   canonical subset of Mazurkiewicz traces of the prepare/commit/observe/obligation task
   system, with wakeup trees seeded by the effect journal's read/write sets. The lab's
   determinism (injected clocks, seeded IDs) makes traces replayable byte-exactly — this is the
   one place in the repo where model checking is cheap, so exploit it. Acceptance: crash-point
   campaign states "explored N traces with DPOR reduction factor R vs enumeration" not "ran
   random tests."
2. **Covering arrays for fault/campaign matrices (BP-6/7).** Disconnect × restart ×
   malformed-frame × epoch-reset × bridge-version campaigns are combinatorial: use
   t-wise covering arrays (greedy AETG/Nordmann-style construction) over parameters
   {fault class, injection boundary, version tuple, interest-set shape} with t=2..3. Gives a
   derived bound: every pairwise interaction of fault conditions exercised at least once, with
   array size O(k log k) instead of k². Each row = one named test deriving from the array, so
   coverage is provable from the array, not asserted.
3. **SSI-style cycle gate at commit (BP: transaction spine stage 6).** The plan already names
   an "SSI-style cycle gate" — make it concrete now: maintain the serialization graph over
   prepared plans (nodes = plans, edges = read-write/write-write/write-read at witness
   granularity), abort-or-rebase on cycle detection at revalidate; hierarchical refinement from
   coarse sound witnesses to fine ones never removes edges that could hide a cycle
   (no-false-negative refinement, fsq-witness heritage). Deterministic outcome: min-cut-free —
   victim selection by canonical plan-digest order (INV-046), so concurrent replays choose
   identical victims.
4. **ATP repair + retrievability with stated bounds (BP: V13, Phase 7).** RaptorQ (RFC 6330)
   systematic symbols for checkpoint/evidence object movement: K source symbols, repair
   overhead ε as an explicit parameter, resume = symbol-level (not object-level); corruption
   detection by symbol-hash; anti-rollback by generation-monotone chain already in `atp.rs`.
   Proof-of-retrievability via Merkle-challenge sampling with hypergeometric confidence: sample
   c of N blocks, corruption fraction f ⇒ detection probability `1 − C(N−fN,c)/C(N,c)`; publish
   (N, c, f, confidence) per audit class instead of ad-hoc spot checks.
5. **Certified top-k attention (BP: V14).** Attention ranking uses a deterministic lexicographic
   scoring semiring (signal-class priority × magnitude × freshness, total order, INV-046
   tie-break) and emits a *selection certificate*: for the returned top-k, a digest proving no
   excluded signal scores above the k-th included one under the declared comparator. Exact
   until TEST-022 budgets prove an approximate policy safe; approximate candidates must
   reproduce the certificate or be rejected (advisory algorithms cannot authorize effects).

## 6. Ambition-round log (in-place revisions)

- **Round 1:** added BP-9 (resource URIs), BP-10 (taint/threat corpus), BP-11 (doc↔registry↔bead
  consistency as mechanism), acceptance-evidence rule, risk register.
- **Round 2:** added §4 interface contracts (CapsulePublisher, FaultSchedule, BridgeHandshake
  FSM, TaskProjection, SessionRegistry), evidence architecture (receipt extension, benchmark
  record enforcement, replay bundle v1, negative-evidence ledger), test-architecture pairing
  rule.
- **Round 3:** added §5 quantitative technique commitments (DPOR, covering arrays, SSI cycle
  gate, RaptorQ+PoR bounds, certified top-k). Bead generation below embeds these so beads never
  need this document for context.

*(Beads generated in §7.)*

## 7. Bead reconciliation and final landscape (Phase 3a + refinement outcome)

**What happened:** the first bead write flushed `.beads/beads.db-wal`, materializing the real
program: 59 closed historical beads (`dfmcp-wp00..wp21`, `dfmcp-wp-{dfh,frk,lea,mcp,pln,tst,wld}*`,
`dfmcp-*`), today's open epics (`df-fastmcp-conformance-5pj`, `df-dfhack-bridge-plane-c-pic`,
`df-franken-storage-mvcc-54h`, `df-action-coordinator-exec-ero`, `df-qualification-pipeline-qqq`),
and two in-progress items (`df-qualification-pipeline-qqq.1` git bootstrap; `dfmcp-k2y` asupersync
0.5 MCP-entry migration, assignee CalmWaterfall, pin target `180a7c8…` ≠ checked-out `12d3469…`).

**Reality-check bead generation:** 41 beads created via `br` from the final bridge plan
(self-contained descriptions with background/reasoning/subtasks/acceptance/registry refs; test
companions where the test effort is a distinct work product). Refinement then reconciled against
the existing graph — never duplicating an owned epic:

- **Closed as duplicates (9), each with a cross-reference comment:** dogfood file/pin-bump/minimize
  superseded by `5pj.1/.2` (after augmenting them); tasks-store binding → `5pj.3`; bridge handshake →
  `pic.1`; payload codecs → `pic.3`; fault campaigns → `pic.5`; qualify receipt → `qqq`; git restore →
  `qqq.1` (in progress); legacy `dfmcp-wp13-modern-handshake-golden-7nu` → `5pj.2` (its id is cited
  in the `#[ignore]` string — update the string when `5pj.2` lands).
- **Augmented (8):** `5pj.1` (+ third era-advertisement defect, capture discipline), `5pj.2`
  (+ pin-bump discipline, retracted-PASS warning), `5pj.3` (+ totality/cancel-guard/indeterminate
  contract), `pic.1` (+ 16 field groups, EvidenceRecord per transition, fail-closed unknown fields),
  `pic.3` (+ golden vectors, bounded decode, digest mutation vectors), `pic.5` (+ covering-array
  methodology with coverage-proof artifact), `tasks-projection-tests` (+ parent reference),
  `session-scoped-capability-negotiation` (+ prior-art pointer to closed `wp13-gate2-session-authority`).
- **Cross-linked (10):** live-observe milestone → `pic.1/.2/.3` + `pub-obs`; `pic.5` → fault-schedule
  harness; dogfood minimization feeds `5pj.1`; noninterference + certified-topk → graph projection;
  registry-v0 → qualification pipeline; asupersync admission → `dfmcp-k2y` (runtime conflict avoidance,
  pin-divergence note).

**Final graph (bv-validated):** 124 total — 53 open, 2 in-progress, 16 blocked (mostly by design:
later-phase beads behind earlier gates), 69 closed, 37 ready. `bv --robot-triage`:
`source_authority.state=complete`, `claim_safe=true`, `readiness=proven`, 124 valid, 0 errors, no
cycles (`br dep cycles`). P0 ready work: bridge handshake + daemon, dogfood minimization,
session-scoped negotiation, bead-graph governance.

**Scorecard (skill format):**

| Claim class | Count | Supported | Overstated | No-evidence-yet |
|---|---:|---:|---:|---:|
| Status-discipline (docs match code) | 14 areas | 14 | 0 | 0 |
| Constitutional mechanics (deps, pin, forbid, validators) | 6 | 6 | 0 | 0 |
| Lab behavior (plan/commit/idempotency/restore/obligation) | 8 | 8 | 0 | 0 |
| Transport lifecycle (modern handshake e2e) | 2 | 0 | 0 | 2 (conformance FAIL recorded; live repro) |
| Gates/SLOs/releases (GATE-010..110, SLO-001..015, DSR) | 15 | 0 | 0 | 15 |
| Bead coverage of vision | phases 0B–3 | covered | — | phases 5–11 un-beaded (sequencing) |

**Recommendation:** (1) keep `df-qualification-pipeline-qqq.1` unblocked and land the git
bootstrap — it gates every receipt; (2) run the dogfood minimization bead and file the three
upstream defects this week — WP-13 closure is on the critical path of everything MCP; (3) wire the
governance bead's coverage checker into `verify.sh` so bead visibility can never silently regress
again; (4) beadify Phase 5–11 only when their predecessors close (the checker enforces the
sequencing).
