# Coherent workforce candidates and capacity planning

The unadmitted spatial/1.8 development server exposes two read-only queries through
`fortress.query`: `workforce_candidates` and `workforce_plan`. They use the strict
citizen roster, sparse skill observations, job-availability flags and terrain from
one coherent capture. The previously disconnected workforce candidate source is
now wired into the actual spatial runtime and schema discovery.

These queries do not assign labor, reserve citizens, create executable plans, or
change the game. They require current Query authority. The production runner map,
compatibility registry, native wire, bridge methods and dependencies are unchanged.

## Inspect candidates for one declared role

Use a native skill key present in the captured citizen skill data. Choose an
observed dry, unoccupied candidate tile inside the session's fixed region as the
target. The coordinates and keys below are illustrative, not a guaranteed match
for another fortress.

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "workforce_candidates",
      "target": [0, 0, 5],
      "skill_key": "CARPENTRY",
      "min_effective_skill": 1,
      "preserve_social": true,
      "adults_only": true,
      "limit": 8
    }
  }
}
```

Candidates must be observed alive, sane and active; adults-only and preservation
of social activities default to true. The selected DFHack job-availability flag
must be true. Effective skill must meet the declared minimum. Candidate ordering
is effective skill descending, nominal skill descending, modeled steps ascending,
then citizen entity ID ascending. Observed stress is exposed but not treated as an
additional optimization objective.

Each row contains a generation-checked citizen handle, observed skill/readiness,
position, terrain approach, and an exact-anchor `map_route` drill-down. A candidate
is not proof of native job eligibility, enabled labor, path feasibility for that
particular unit, job readiness or safety.

The candidate query's default minimum is zero. Sparse missing skills are treated
as zero only when that skill key is represented elsewhere in the same capture.
A key absent from the entire capture produces `skill_key_not_observed` exclusions
and `skill_key_observed_in_capture=false`; it does not recruit every novice. This
also prevents a misspelled key from silently matching everybody. The sparse roster
is not a complete native skill-key registry, so absence can be inconclusive.

## Allocate people across simultaneous demands

Independent candidate lists can name the same person repeatedly. `workforce_plan`
solves all declared demands together with capacity one per citizen.

```json
{
  "session_id": "<spatial session>",
  "query": {
    "schema": "dfmcp.query/1",
    "query": {
      "kind": "workforce_plan",
      "demands": [
        {
          "key": "carpentry",
          "workers": 2,
          "target": [0, 0, 5],
          "skill_key": "CARPENTRY",
          "min_effective_skill": 1
        },
        {
          "key": "mining",
          "workers": 1,
          "target": [2, 0, 5],
          "skill_key": "MINING",
          "min_effective_skill": 1
        }
      ],
      "limit": 8,
      "max_work": 10000000
    }
  }
}
```

A request contains 1..16 uniquely keyed demands and at most 128 worker slots in
total. Each demand requires a key, positive worker count, target and skill key.
The plan query defaults its effective-skill minimum to **one**, unlike candidate
inspection's zero default. The social and adulthood policies default to true in
both queries. Unknown fields, duplicate keys, excessive counts, unsupported target
cells and invalid coordinates are refused.

The existing integral allocation solver receives one unit of supply per citizen
and an eligibility mask across canonically sorted demand keys. Residual rerouting
can place a flexible worker on the role only they can fill while assigning a
specialist to the other role. One citizen cannot fill two simultaneous slots.

The objective is **maximum filled slots**, not maximum aggregate skill, minimum
travel, deadline scheduling or priority-weighted allocation. Ties are deterministic
under eligibility-mask, demand-key and citizen-ID ordering. Candidate preference
ranks are not a claim of globally optimal assignment quality. No reservation is
created, so another request may independently consider the same people.

The result contains:

- `model_feasible`, requested/assigned counts, worker capacity and primal/cut agreement;
- per-demand requirements, assigned/unfilled counts, candidate counts and disjoint
  primary exclusion classifications;
- whole assignment rows with identity, observation evidence and route drill-downs;
- when infeasible, one Hall-type deficient subset with required workers, distinct
  candidate workers, their IDs and the deficit.

The shortage is about the declared observed model. Overlapping demand shortages
are not independently additive, and the certificate does not prove a fortress-wide
labor shortage. Excluded terrain, unsupported navigation, unobserved skill keys or
non-citizen workers can limit this model without proving global impossibility.

## Terrain endpoint semantics

Targets must be observed candidate tiles, not occupied workshops. Ordinary routes
use the existing dry cardinal floor/complementary-stair model. A standing citizen
may occupy an otherwise eligible tile; for that endpoint only, the analysis can
model a single horizontal step to a reachable neighboring candidate tile. The row
marks that assumption with `unit_endpoint_step_modeled=true`.

Only unit occupancy is relaxed. Hidden/unallocated terrain, walls, liquids,
building occupancy, ramps and unknown walkability at the citizen endpoint do not
become routes merely because a neighboring floor is reachable. The returned route
ends at `approach_tile`; the separately modeled occupied-endpoint step is not
misrepresented as a native unit-navigation proof.

## Anchors, budgets and active watches

These operations read the session's latest retained live capture without acquiring
a new native observation or controlling game time. Freshness is the capture time,
not the time a later page is returned. A fenced source is refused, and the exact
supplied `expected_anchor`, when present, must agree with the session.

The solver is run on the complete bounded model before assignment rows are paged.
Pagination does not allocate page by page or consume workers. The model digest
binds normalized demands, policy, full anchor and source digest. Continuations
bind the session, model and work allowance: `wc2` for candidates and `wp1` for
plans. Prior `wc1` tokens are not accepted under the stricter endpoint policy.
Changing page width is allowed; changing capture, session, demand, policy or work
allowance requires restarting the result.

Page limits are 1..128. A candidate query permits at most 1,000,000 work units; a
plan permits at most 10,000,000. Multiple demands at the same target share one
reachability computation. Skill scanning, route work, candidate ordering and
allocation share the bounded allowance. Work exhaustion is an error, not a false
shortage. Wall-time checks are cooperative, not a hard preemption guarantee.

The actual MCP path reserves the complete Agent Turn and current active watches
before filling the result page. Complete model summaries and whole rows must fit;
a budget too small for one row is an explicit failure rather than a zero-progress
continuation. Repeating a page does not mutate the model. Current watches remain
attached without being sampled or advanced by workforce analysis.

Schema discovery (`fortress.query` with `mode="schema"`) includes both query
variants. Workforce queries are currently live-capture analysis only and are not
accepted inside `historical_query`. Existing historical queries and the separate
pause-control profile are unchanged. The top-level `fortress.plan` and
`fortress.commit` tools still refuse game effects in spatial/1.8.

## Validation status

Fifteen new Rust tests are registered: eight adapter analysis/allocation tests,
four query contract/pagination tests and three actual spatial MCP-handler tests.
The adapter suite includes all 512 three-worker/three-demand skill graphs checked
against an independent exhaustive assignment oracle. Other scenarios cover
shared-worker conflicts, multiple worker slots, shortage neighbor counts, unknown
skill keys, occupied endpoints, authority and work limits. Handler scenarios cover
schema discovery, route drill-downs and complete 8,192-byte pagination with current
watches. **These Rust tests have not been compiled or executed in the editing
environment**, which has no Rust compiler, Cargo or rustfmt.

The checked-in Python reference `scripts/test_workforce_query_contract.py` was
executed successfully: 88 schema cases (25 accepted, 63 rejected), plus 4,096
independent mathematical oracle cases comparing exhaustive unit-capacity worker
assignments against deficient-subset bounds. Both new query schemas also passed
Draft 2020-12 schema validation. The executed script SHA-256 is
`ff87fdd47943ad6b7dc88ec9cec7148fb0490280cf8ff597995973c050268d04`.

Those Python oracles do **not** execute the Rust allocator, terrain analysis,
serialization, MCP handlers or DFHack. Schema validation does not establish
runtime authority, UTF-8 byte limits, duplicate-key rejection or the aggregate
128-worker bound; those are implemented and covered by registered Rust tests.
No Rust pass, Clippy, stdio, native build, live fortress campaign, full repository
qualification or production admission is claimed.

Focused validation commands on a configured checkout:

```bash
cargo test --locked -p dfmcp-adapter workforce_analysis
cargo test --locked -p dfmcp-mcp workforce_queries
python scripts/test_workforce_query_contract.py
```

These commands do not replace the repository's full verification and qualification
requirements. Complete native labor configuration, actual labor assignment,
unit-specific pathfinding, historical workforce analysis and quality-weighted
multi-role scheduling remain outside this increment.
