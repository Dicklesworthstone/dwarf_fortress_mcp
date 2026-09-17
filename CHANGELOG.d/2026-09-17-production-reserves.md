# Protected stock in joint production portfolios

## Implemented source

- Extend the existing spatial/1.8 production_portfolio query with optional hard
  reserve pools. Every candidate task set must support all reserves using the
  same conservative, same-capture, route-aware material model as task inputs.
- Give reserves no task priority and never relax them to improve production score.
  Reserve support is solved jointly with each candidate, not greedily subtracted
  from particular stacks. Shared capacity prevents double-counting across tasks
  and pools; overlapping reserve selectors still require DISTINCT stock units.
- Preserve full input validation before subset exclusion and reserve-only
  infeasibility checks. Support at most eight pools and 32 combined task-input
  and reserve demands. Raw selector cardinality is checked before deduplication.
- Distinguish feasible empty production with supported reserves from infeasible
  hard reserves. A checked reserve-only deficient-subset witness excludes all
  task sets, including the empty one; no feasible optimum or partial production
  allocation is reported in that case.
- Separate production consumption from unconsumed reserve support in response
  totals and rows. Protected model stock is not a native reservation or proof
  that the game's current stock is held, accessible or globally sufficient.
- Expose whole reserve-assignment and reserve-shortfall rows through existing
  complete-packet pagination. Normalize and bind nonempty reserve definitions
  into model/continuation identity. Preserve the original no-reserve model
  identity for omitted, null and empty reserve lists.
- Reuse the actual live, historical and offline production dispatchers. Reserve
  route drill-downs remain pinned to their exact historical record. Current
  watches and journal bytes are unchanged; no extra capture or game effect runs.
- Keep the original typed plan API as a no-reserve wrapper and add the explicit
  plan_with_reserves API. Schema discovery advertises the same material-selector
  contract for task inputs and reserves.

Usage and semantics: docs/PRODUCTION_RESERVES.md. This fragment supplements the
implementation/evidence status; the formal unadmitted phase is unchanged. Native
protocols, journal formats, dependencies, top-level tools, production runners,
compatibility registry and mutation authority are unchanged. This does not add
workshop capacity, time schedules, future outputs, task dependencies, item locks
or automatic execution.

## Validation

Eighteen new Rust test functions are registered: seven public selection tests,
five coherent adapter tests and six actual MCP-handler/private-file tests. They
cover reserve priority, joint rerouting, shared-stock cuts, empty and infeasible
models, normalization, input bounds, existing API compatibility, response budgets,
pagination, unchanged watches/history, current authority and historical/offline
route binding. One test contains 8,192 independently enumerated small allocation
graphs. These are registered but UNEXECUTED Rust cases.

No Rust compilation, Cargo tests, rustfmt or runtime execution occurred in the
editing environment. Those tools are absent; container access to obtain a
checkout/toolchain is unavailable. Source/diff review and GitHub branch/commit
verification do not establish Rust, MCP, native/live-game, filesystem-crash,
full-repository qualification or admission evidence.

The executed request-schema checker passed 68 cases: 14 accepted and 54 rejected.
Each was validated both directly and inside a query envelope, for 136 checks.
Task-input and reserve selector schemas were also checked for equality.
Tested schema Git blob: 8ef34941c00ff021b7f09e2fb8179c08ddd1be26.
Tested script Git blob: d5613f42f083a6de51c38668f32d73756abded83.
Both match the committed bytes. Script SHA-256:
a783e2b5df5622759a9168f22bb7ee7760b171e43c1b03e890603e42ad3638e2.
These checks cover request structure only, not duplicate keys, aggregate limits,
source state, allocation, query execution, custody or historical replay.
