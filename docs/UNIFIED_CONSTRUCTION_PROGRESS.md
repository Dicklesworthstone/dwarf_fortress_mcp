# One construction-progress entrypoint through the final placement

The existing `route_furniture_progress.py review`, `progress` and `query` commands
now support both next-placement and final-construction clock scopes. No new native
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
The common Agent Turn now includes final-construction sampled quiescence without
claiming stability-qualified completion. Stale next-step consent cannot start a
final effect; the final owner still reacquires and validates its separate consent.

Direct script execution aliases its own module before invoking the CLI. This
prevents the final owner's import from constructing a second incompatible
ProgressAuthority class. The authority requirements themselves are unchanged.

Executed development evidence:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=scripts python3 -m unittest \
  test_progress_family_routing test_progress_script_entrypoint -v
```

All 18 tests pass. Both complete production owner modules execute without source
rewriting, including CLI/main and a runpy `__main__` execution. Tests reproduce the
old wrong-key/wrong-family result and cover final review, one-effect delegation,
lost-reply query flow, old-scope preservation, history refusal and authority loss.
Native clients, scope/source codecs, original storage/budget, acquisition and outer
packet dependencies are explicit test ports: this is routing/workflow evidence,
NOT native TCP, private-file, DFHack, Rust/MCP or full-workspace qualification.
Beads df-dfhack-bridge-plane-c-pic.4/.5 remain open.
