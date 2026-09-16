# Observed item quantities and stock threshold monitoring

## Implemented source

- `item_quantity` through the existing query dispatcher and as a watch success
  or failure condition. It sums native U64 stack sizes, not item-record counts.
- One measurement implementation for inspection, single watches and shared-capture
  watch batches. Predicates, definition limits, scan accounting and three-valued
  membership semantics are reused from population watches.
- Explicit quantity bounds: uncertain membership contributes zero through known
  quantity; missing quantity leaves no established upper bound. False memberships
  need no quantity evidence. Zero quantities remain zero. Overflow fails rather
  than wrapping, saturating or returning partial evidence.
- Whole bounded measurement and scoped examples with snapshot/predicate evidence,
  Query/anchor/integrity checks, response budgets and cooperative deadlines.
  Raw stack totals do not prove usable supply, food portions, nutrition, inherited
  container eligibility, reachability, native requirements or complete-world absence.
- Existing durable watch serialization and restart behavior retain definitions
  under fresh handles, reset unfinished stability and require fresh samples.
  Stack splits/merges do not change an otherwise equal quantity threshold.
- Stateless quantity queries against current, exact historical and offline archive
  captures without observing the bridge or advancing current watches. Historical
  facts remain historical. Archive-only watch evaluation stays unavailable.
- Shared schema composition registers the new condition/query once and retains
  population, workforce, history and live batch definitions. Archive discovery
  has thirteen stateless variants and three history variants.

The formal unadmitted phase in IMPLEMENTATION_STATUS.md is unchanged. No native
wire format, dependency, production runner, compatibility entry, top-level tool,
background task or game-effect authority was added. The implementation status and
usage of this increment are documented in `docs/ITEM_QUANTITY_MONITORING.md`.

## Evidence and limitations

Fifteen new Rust tests are registered: ten measurement/watch tests and five actual
spatial-handler tests. Existing archive/schema regressions are extended, not
removed. None has been compiled or executed here: rustc, Cargo and rustfmt are
unavailable, and direct toolchain-host access failed.

The checked-in Python reference passed 520 schema cases (475 accepted, 45 rejected),
6,552 finite-interval cases and 124,410 comparison checks over 1,885 population
models, plus overflow and large-integer checks. This is scoped contract/math
reference evidence only, not execution of Rust, the complete MCP envelope,
runtime schema composition, state recovery, filesystem publication or DFHack.
Unknown-quantity completions use bounded adversarial values, not infinite
enumeration. The executed script and two extension files match their Git blobs.
Script SHA-256: `a02955780045531726d09b544f4db2ba17ea2b016064591cd14bdb49b17084dc`.

No Rust, native, live-game, repository or production qualification is claimed.
