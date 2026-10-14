# Bounded clock progress after the final furniture placement

After every original placement receipt exists there is no next placement for the
ordinary route frontier to review. `route_furniture_final.read_frontier` can
observe all original construction, but before this increment a pending final
construction had no scope-bound way to advance game time.

`scripts/route_furniture_final_progress.py` closes that execution gap without
changing existing version-2 next-step confirmations. It has a distinct
`dfmcp.route-furniture-final-progress/1` scope and `frf-` native key family.
Both old and final scopes live in the same private clock-history directory, and
the existing complete history gate must report every earlier effect resolved
before a new final effect can create scope or native intent.

`review` brackets one exact read-only run/1.13 observation with equal complete
final-construction frontiers. Only `construction_pending` returns a confirmation.
Quiescent, blocked or unestablished final observations do not authorize a clock
effect. The confirmation binds the complete original batch inventory, original
plan/review, final evidence, source, exact clock observation, and tick/wall limits.

`progress` reacquires that confirmation, synchronizes its immutable association,
rechecks all prior history, and uses the unchanged run/1.13 prepare/commit path at
most once. Scope, original batch custody, prior history and synchronized intent
are checked again at PrepareRun, CommitRun and publication. There is no automatic
clock loop, furniture dispatch, retry, replacement item, or new native method.

After a pause-verified stop, the workflow samples the complete final construction
frontier again. `all_original_furnishings_quiescent_now` is only a sampled
condition. It does not become construction completion, stability, usability, or
effect discharge. The existing stability-qualified completion workflow remains
the authority for those stronger claims.

`query` is query-only recovery of the exact original final key. A retained
terminal result is inspectable offline. A regular next-step progress confirmation
cannot be reinterpreted as a final-construction scope, and vice versa. Existing
`route_furniture_progress.py inventory` discovers both families because they
share the same history owner and directory fence.

This source increment is development-only. It adds no production admission,
compatibility tuple, MCP tool, native protocol or journal rewrite. Focused source
regressions are checked in; live DFHack, Rust/MCP and full-workspace qualification
remain separate.
