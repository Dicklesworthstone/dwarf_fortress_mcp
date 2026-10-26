# One construction-progress entrypoint through the final placement

The existing `route_furniture_progress.py review`, `progress` and `query` commands
support both next-placement and final-construction clock scopes. No new native
method, clock loop, placement permission, journal generation or admission is added.

For new work the held original batch chooses the existing owner. An all-placed
batch uses `route_furniture_final_progress` and its complete original-receipt
frontier; an unfinished batch keeps the existing next-step path. Final reviews
also expose `progress_review`, with the identical final scope and confirmation,
so callers do not need another confirmation lookup. The complete response is
bounded again after that additive handoff. Final blocked, unknown and quiescent
states return no clock consent. No failed final read falls back to a next step.

Recovery chooses the family from the validated retained scope, not current batch
status. A discovered final record retains its `frf-*` key and final-construction
report, and an explicit refresh uses the complete final reader. Old `frp-*`
records stay original next-step history even after every placement is registered.
Offline terminal inspection remains offline; uncertain recovery remains query-only.
The common Agent Turn includes final-construction sampled quiescence without
claiming stability-qualified completion. Stale next-step consent cannot start a
final effect; the final owner still reacquires and validates its separate consent.

Direct script execution aliases its own module before invoking the CLI. This
prevents the final owner's import from constructing a second incompatible
ProgressAuthority class. The authority requirements themselves are unchanged.

## Inspect final construction without clock-control credentials

After every original placement is registered, use the existing isolated route
read environment (the route opt-in, endpoint and furniture/map/operations read
credentials) to request a fresh report:

```sh
python3 scripts/route_furniture_progress.py observe-final \
  --directory /absolute/private/route-batch --batch-id <original-batch-id>
```

This operation needs no run/1.13 token, clock confirmation, tick/wall selection or
clock-history directory. It uses the existing `route_furniture_final.read_frontier`
reader, including its complete original-receipt checks. It does not open a clock
connection, create a clock record, grant placement, or advance simulation time.
Clock/paging overrides are rejected before opening custody. A failed acquisition
is not retried. Final authority, custody, deadline and output-size checks remain;
the original batch stays open through the single output attempt.

The report retains the full original batch and complete final frontier. Agent
Turn shows `construction_pending`, `construction_quiescent`, `construction_blocked`
or `construction_unestablished`. Current sampled quiescence is not stable goal
completion, ongoing usability or an effect-retry permit. All placement-confirmation
fields remain null and `advance_allowed` remains false.

## Executed evidence and limits

```sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_progress_family_routing.py --mutations
```

All 29 routing/CLI tests pass, including 11 final-inspection cases, and four
weakened implementations fail assertion-based regressions. Both complete production
progress owners execute without source rewriting, including main and runpy
`__main__` execution. The exact preceding source reproduces the wrong-family/key
recovery result. Legacy next-step review/query fixture packets are byte-identical.

Native clients, scope/source codecs, original storage/budget, acquisition and outer
packet dependencies are explicit test ports. This is routing/workflow evidence,
NOT native TCP, private-file, DFHack, Rust/MCP or full-workspace qualification.
The source-hashing runner measures all six executed production/test inputs.
Beads df-dfhack-bridge-plane-c-pic.4/.5 remain open.
