# Staged spatial observation acceptance

## Implemented source

- Replace the spatial/1.8 runtime's inline refresh with a shared acceptance
  boundary used by observe, wait, single-watch await and batch-watch await.
- Fix the nonjournaled path publishing a new world before checking authority at
  the new game tick. Build an unpublished typed candidate, including generation
  history, and require target Observe authority before either storage path can
  make it current. Prior anchors cannot authorize admission of newer facts.
- Require target Query authority for journaled captures as well as Observe.
  Preserve Observe-only sessions without persistence. Validate the actual
  projected entity count alongside existing capture/roster/region limits.
- Recheck current journal custody before acquisition, not merely its cached
  health/anchor, and again before publication. Changed archives fail closed;
  no captured world is appended after detected custody loss.
- Share the refresh allowance across source acquisition, candidate validation
  and entry to publication. Pass only remaining time to source and journal;
  reject late injected/slow results instead of renewing the entire allowance.
- Leave the old world, source digest, cursor and generation history unchanged
  on candidate refusal. Fence a consumed-but-rejected source so current live
  queries cannot present that old world as a healthy current source. A refusal
  before source entry does not unnecessarily poison a healthy connection.
- Preserve journal sync-before-publication and uncertainty handling. No fallible
  post-sync timeout check hides a committed root. Observation and watch storage
  remain separate transactions; a later watch/render failure can still follow
  an accepted observation without rolling it back.
- Wire source bootstrap to one connection/first-capture/validation allowance.
  Failed unpublished sources are dropped before archive/watch startup and before
  session registration. The existing opening slot guard releases capacity.

Details and limits: docs/SPATIAL_OBSERVATION_ACCEPTANCE.md. These are cooperative
source-operation deadlines, not hard preemption or a claim that subsequent
archive replay, watch recovery and rendering share one entire-open deadline.
Staging adds bounded projection work; no performance qualification is claimed.
Historical/local evidence keeps its existing authority rules; this is not a
continuous external authorization clock. Native protocols, journal formats,
request schemas, dependencies, top-level tools, game effects and production
admission are unchanged. This fragment supplements source/evidence status; the
formal unadmitted phase in IMPLEMENTATION_STATUS.md is unchanged.

## Validation

Fifteen new Rust test functions are registered: eight acceptance/private-file
scenarios, four deterministic bootstrap deadline/cleanup scenarios, and three
actual MCP-handler scenarios. Coverage includes target Observe/Query expiry,
unchanged rejected roots, pre/post-read deadlines, reduced source allowances,
projection/roster/region limits, corruption before/during capture, caller and
session fences, heartbeat/reset behavior, unchanged single/batch watches after
refusal, and successful shared-watch capture. No timing sleeps were introduced.

NONE of these Rust tests has been compiled or executed here. Rust, Cargo and
rustfmt are unavailable, and container DNS access to repository/toolchain hosts
failed. Validation consists of source review and GitHub diff/commit/branch
verification. No Python model is offered as a substitute for execution of these
Rust paths. No native DFHack, live-game, filesystem-crash, full-repository or
production-admission qualification is established.

Focused command:

    cargo test --locked -p dfmcp-mcp 'observation::' -- --test-threads=1
