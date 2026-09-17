# Historical resource timelines

## Implemented source

- Add historical_series through the existing spatial/1.8 fortress.query history
  dispatcher, with live and offline archive schema discovery. Select one exact
  inclusive record interval and the existing stateless item_quantity measurement.
- Reuse the same quantity evaluator as current inspection and quantity watches.
  Each row retains the exact source/record/anchor witnesses, conservative bounds,
  ordinary measurement evidence digest, and adjacent-sample net change.
- Add ObservationJournal::project_records: reconstruct one verified prefix per
  page, retain only bounded projected values, and keep the current world and
  generation history unchanged. Validate the whole exact selection first;
  verify unselected prefix records too. Late failures return no partial result.
- Support 1..32 whole samples per page plus one internal preceding sample. Keep
  cross-page changes identical to an unpaged result. hs1 continuations bind the
  current session, archive identity/head, interval and measurement, but permit
  page width/output budget changes. No empty-progress continuation is emitted.
- Preserve open quantity bounds instead of manufacturing exact deltas. Use exact
  signed decimal numerators and game-tick denominators for endpoint net rates.
  Same-tick samples have no rate; epoch/clock discontinuities have no cross-boundary
  change. Equal uncertain intervals do not establish unchanged stock.
- Reserve complete Agent Turns, fixed range evidence and current watches before
  replay; check complete output again. All page phases share a cooperative wall
  allowance. Each of at most 33 measurements retains its own existing one-million
  work-unit limit, and internal measurement output is capped at 16 KiB.
- Recheck current authority and archive custody before/after replay. Historical
  ticks cannot revive expired grants. Healthy archived reads remain available
  after the live source fails; reopening uses new handles and invalidates old
  page tokens. The selected interval can cross resets, but rates never do.
- Keep watches, baselines, observation bytes and current state unchanged. No bridge
  call, capture, journal append/repair, reservation, effect or background work is
  added. Recursive history and stateful measurements remain refused.

Usage and limitations: docs/HISTORICAL_RESOURCE_SERIES.md. This is a timeline of
retained observation samples, not continuous event history, usable-stock proof,
a causal production/consumption model, a forecast or current game state. A partial
page does not claim its unvisited suffix was reverified. Replaying later pages
still requires their generation-history prefix; no random-access, peak-memory or
performance qualification is claimed.

Native protocols, journal formats, dependencies, top-level tools, game-effect
families, compatibility registry and production admission are unchanged. This
fragment supplements implementation/evidence status; the formal unadmitted phase
is unchanged.

## Validation

Twenty-one new Rust test functions are registered: five journal projection tests,
eight mathematical/boundary tests and eight actual MCP-handler/private-file
scenarios. The interval test contains 1,296 exhaustively enumerated interval
pairs. Coverage includes exact individual-query equivalence, one-prefix read
counts, unchanged worlds/watches/journals, cross-page differences, response bounds,
current authority, uncertain/full-width arithmetic, same-tick/reset behavior,
source failure, offline reopening, stale continuations, schema composition and
same-size corruption in an unselected prefix. The existing archive schema test
retains its prior fourteen single-record read kinds and adds timeline discovery.

NONE of those Rust tests has been compiled or executed here. Rust, Cargo and
rustfmt are absent, and direct container DNS access to GitHub failed. Validation
consists of source review and GitHub diff/commit/branch verification. There is no
Python mirror presented as execution of the Rust paths, and no passing Rust,
MCP, native/live-game, filesystem-crash or full-repository qualification claim.

Focused commands:

    cargo test --locked -p dfmcp-adapter 'file_storage::projection::tests' -- --test-threads=1
    cargo test --locked -p dfmcp-mcp 'history::changes::series::' -- --test-threads=1
