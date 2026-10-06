# Inventory acquisition and local projection have separate lifetimes

Beads: `df-dfhack-bridge-plane-c-pic.3` and `df-dfhack-bridge-plane-c-pic.5`.

The furniture handoff reader now owns its native TCP link inside a private
`read_observation` boundary. That boundary performs the existing handshake,
complete paged acquisition, digest/source/authority checks and verified release.
Only the owned decoded observation crosses back to `acquire_trusted`; the
connection has already closed before canonical graph construction begins.

This matters for the maximum operations/1.4 roster: 4,096 jobs, 4,096 buildings
and 65,536 items, plus the fortress root. Local graph construction must not
retain an idle native connection after the source's capture has been released.
The request's existing `Work` remains alive, including its original deadline,
high-water game tick, cancellation and operator permission. Checks before and
after publication are unchanged. No reconnect, repeated read, mutation,
authority renewal, new wire protocol or production admission is introduced.

The lifecycle regression uses a joined TCP peer and joins it **before** local
projection. It requires EOF after the original release and separately exercises
normal publication, cancellation, operator revocation and deadline exhaustion.
It does not depend on making graph construction slower than a socket timeout.
Existing maximum-roster and malformed/source/release/cancellation tests remain.

## Execution evidence

At parent revision `2bc8af17b55c05f50a3ccecfef1f4cb76e701cff`, the exact pinned
nightly-2026-08-31 adapter build completed. The `furniture_handoff` filter ran
39 tests: 38 passed and the maximum-roster test failed with
`allocation TCP test peer failed`. This established a failing baseline, not a
passing result for this change. Retaining the link through projection was
identified by source inspection; the generic peer error alone does not establish
the precise point at which its timeout occurred.

The execution connection then became unavailable while a separate MCP binary
build was running. Its final result could not be retrieved. **This change and
its new regression have not been compiled or executed.** No pass, latency
improvement, live DFHack qualification or production support is claimed.

Required follow-up on the pinned toolchain:

```sh
CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --locked --offline -p dfmcp-adapter --lib furniture_handoff -- --test-threads=1
```

The inventory handoff and whole-plan completion MCP process suites still need
to execute against a successfully built furniture development server. This note
supersedes neither their source-only status nor the empty admission registry.
