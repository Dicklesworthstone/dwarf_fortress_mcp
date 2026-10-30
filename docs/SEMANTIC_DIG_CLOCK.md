# Finite advancement of an original mining goal

The original mining development server can optionally join its exact retained
Mine designation to independently authorized run/1.13 clock custody. Each
interval requires a new plan, a distinct review and explicit confirmation. This
remains an unadmitted development profile; source and component tests do not
establish native/live qualification or a complete qualified FastMCP executable.

## Configure the independent clock

Use the original source, scope, checkpoint and private-directory setup in
[the mining guide](SEMANTIC_DIG_DEVELOPMENT.md). At initialization add the
optional top-level configuration member `"clock":{"generation":9}`, replacing
`9` with the independently observed run/1.13 incarnation. Set
`DFMCP_SEMANTIC_DIG_ALLOW_CLOCK=1` and provide that source's credential through
`DFMCP_SEMANTIC_DIG_CLOCK_TOKEN`. Neither clock authority nor its credential can
be supplied through an MCP argument. Original designation authority stays
independent. Clock configuration creates a separate private `clock.journal`.

The clock shares the configured endpoint and pinned software versions. Its
protocol identifies an incarnation, endpoint and software, **not a named world
folder or site**. Matching ticks do not supply the missing fortress identity.
The operator must independently accept this source; the named-fortress terrain
policy cannot be converted into native clock authority.

Open with a positive `max_game_ticks`, at most 1200. The default is zero; opening
never unpauses. When no `max_bytes` is supplied, a positive clock allowance uses
3 GiB of logical byte-work instead of the designation-only 2 GiB default. This
funds both fresh terrain checks for a short retained history, not an allocation
of that much memory. Explicit byte limits are always respected. Longer history
may require a larger explicit allowance, up to the existing 64 GiB ceiling.

## Review and execute one interval

First observe, plan and explicitly confirm the original Mine rectangle. Keep
the paired original/native seal. After its verified `Designated` receipt, wait
to initialize original goal history and obtain a fresh source-qualified sample.

The optional `clock_readiness` result qualifies the exact original target.
Visible dry walls retaining the original Mine designation, together with dry
cleared floors, may be eligible. Hidden, missing, wet, building-occupied, changed
or no-longer-designated walls block a new run. A pending wall's canonical floor
truth remains **unknown**. Readiness establishes neither reachability nor
structural safety, and never upgrades excavation completion.

Pass the following inner JSON as the `request` string to `fortress.plan`:

```json
{
  "kind": "run",
  "original_paired_seal": "EXACT_ORIGINAL_64_CHARACTER_PAIRED_SEAL",
  "game_ticks": 10,
  "wall_millis": 1000
}
```

The interval must fit the negotiated budget, original polling cadence, unchanged
absolute deadline and native run limits. Preparation acquires fresh paused
original-goal evidence and returns a distinct `review_seal`; it does not unpause.
Review that interval and call `fortress.commit` with `scope="run"`,
`paired_seal=review_seal` and `confirmed=true`. The default `scope="goal"` still
confirms only the original designation. The two seals are not interchangeable.

The coordinator records durable intent before one native unpause attempt. After
that marker, the dispatch boundary rechecks exact original custody, the verified
designation, freshly qualified same-tick terrain and independent clock authority.
A failed or lost reply is reconciled under its original clock key, never retried
as another unpause. A known insufficient work allowance refuses before native
clock contact. After a fresh sample, additional retained-history work must also
fit before preparation or unpause.

`fortress.wait` first reconciles retained clock work. Only a settled outcome
allows a fresh paused excavation sample at or beyond the retained clock floor.
A historical stopped receipt cannot make an older sample fresh. Further time
requires another separately reviewed interval. Wait never starts or chains runs,
and same-tick samples cannot manufacture original temporal stability. At most
64 intervals are retained without pruning or replacing original history.

## Cancellation, restart and evidence

`fortress.cancel(scope="run")` retires an undispatched interval or asks its native
owner for the safety pause. Current independent clock authority is required;
fresh goal truth, Plan/Observe/Designate grants, a future goal deadline and a
positive future tick allowance are not. Cancellation does not undo elapsed time.
Session release does not claim that the game has paused.

On restart retain the same clock generation and exact clock journal identity.
Set `clock.journal_id` to the returned `clock_journal_id` and, once an original
archive exists, keep that same ID in its complete `recovery` object. Initialization
creates new custody; reopening requires exact existing custody and never creates
a replacement journal. Even a clock initialized before an original goal needs
its exact identity to reopen. Missing, substituted or unexpected custody is
refused.

Offline and reconcile modes never unpause. Reopening in control with independent
clock authority can reconcile or continue the already verified designation;
it cannot recover the original designation permit. Current excavation truth is
withdrawn on restart and unfinished stability is interrupted, while historical
first achievement remains separate. Unresolved clock effects suppress current
proof and new runs. Every failure, oversized response and session handoff keeps
the exact original and clock identities without retaining control authority.

A terminal unresolved run, such as permanent native source loss, remains
indeterminate. Repeated waits or cancellation of that terminal record cannot
resolve it or establish a safety pause. Preserve its exact handoff and release
local custody; the packet does not claim that the game stopped.

Historical designation, clock coordination, current excavation proof and
historical first achievement are distinct claims. Generic action DAGs,
multi-region mining, checkpoint/restore, automatic run chaining and exclusion
of other game controllers remain outside this slice.
