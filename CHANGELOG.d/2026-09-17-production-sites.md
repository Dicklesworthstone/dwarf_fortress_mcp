# Location-aware joint production planning

## Implemented source

- Extend the existing spatial/1.8 production_portfolio query with optional
  per-task origins. Omitted/null values inherit the top-level default; every
  hard reserve remains anchored to the default origin.
- Feed task-specific targets into the existing coherent workforce analyzer.
  Each citizen retains capacity one across every selected site and task.
- Reuse conservative spatial inventory analysis once per distinct site, retaining
  complete candidate pools rather than local partial allocations. Restrict local
  eligibility to the demands located there, then join by stable stack identity.
  A stack reachable from several sites retains its capacity, never its sum.
- Solve one global all-or-nothing task selection with mandatory reserves. Stock
  or workers in a disconnected district cannot satisfy a task elsewhere. Remote
  inaccessible stock cannot conceal a default-site reserve shortfall.
- Preserve exact same-capture anchors/source identities, inherited container
  exclusions and existing candidate-route restrictions. Validate sites even for
  tasks that lose on priority. Invalid/excluded sites fail the whole request.
- Expose resolved task/assignment origins, demand-aligned origins on shortage
  evidence, distinct global candidate counts and explicitly nonadditive site
  summaries. Route requests and step counts use the assignment's actual site.
- Continue through the existing live, historical and offline dispatchers. Exact
  historical record pinning retains site coordinates, and current watches and
  journal bytes are not changed or sampled. No extra native capture is made.
- Bind nondefault origins into model/continuation identity under an explicit
  multisite policy. Omitted/null/explicit-default origins retain the original
  common-origin identity. Existing typed APIs wrap the new plan_at_sites API;
  ProductionTask constructors do not change.
- Retain shared work/time/output limits, at most eight tasks/nine distinct site
  reports, 128 worker slots, eight reserves, 32 material demands and 32,768
  combined candidate stacks. Exhaustion never returns a partial optimum.

Usage and limitations: docs/PRODUCTION_SITES.md. Separate bounded site reports
may repeat material scans; this is not a performance or peak-memory qualification.
No native workshop capacity, hauling schedule, carrying assignment, dependency
schedule, future output, reservation, labor assignment or game effect is inferred.
Native protocols, journal formats, dependencies, production runners, compatibility
registry and mutation authority are unchanged. This fragment supplements the
implementation/evidence status; the formal unadmitted phase remains unchanged.

## Validation

Seventeen new Rust test functions are registered: two shared-pool tests, eight
coherent adapter integration tests and seven actual MCP-handler/private-file
scenarios. One adapter test enumerates 256 small independent site-capacity cases.
Coverage includes disconnected sites, shared-stack limits, default-site reserves,
wrong-location shortages, schema discovery, normalized defaults, paging and stale
continuations, exact historical routes, offline recovery, authority/budget refusal
and unchanged observations/watch evidence.

NONE of those Rust tests has been compiled or executed here. Rust, Cargo and
rustfmt are unavailable. Container DNS access to the toolchain host and repository
failed. Source review and GitHub diff/branch verification do not establish a
passing Rust build, live MCP, DFHack, crash durability or production admission.

The executed Python request-schema checker passed 60 cases: 12 accepted and 48
rejected, each checked directly and inside an envelope (120 checks). It also
verified equality of task-material and reserve selector schemas. Tested schema
Git blob: 6d39942f06a852ca734f2a43f4509a8a9b45a5f5. Tested script Git blob:
b47b09dd7cb01dd2371856d85094329a2a42abca. Both match the committed bytes.
Script SHA-256: 8f469e871a22c496edac71630f04ae3e131121627d784c8dfa65f7fb21611890.
Those checks cover request structure only, not site existence, routes, allocation,
aggregate work limits, Rust serialization, MCP publication or archive replay.
