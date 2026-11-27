# Completed original actions remain visible after recovery

Owners: `dfmcp-reality-wiring-proof-4bf.11` and
`df-lab-selected-plan-wait-s4q` (broader acceptance remains in progress).

Completed recovered steps previously disappeared from passive work inspection
and original-goal Explain after their live handles and carried monitors were
gone, even when exact atomic completion receipts survived in the durable
archive. A zero-tick selected wait could inspect that history, but a passive
reader could not.

A borrowed archived-disposition view now exposes the already-verified complete
step map. Verification captures the exact typed original object digest as
process-local metadata. Work and Explain must admit that same typed archive
plus the original semantic digest and ordered steps before reading the map.
Existing custody digests and durable journal bytes are unchanged.

Work catalogs and detail resources include missing historical steps; live and
carried records take precedence for the same original plan and step. Explain
step pages use the same fallback. Rows preserve recorded and recovery anchors,
original disposition and custody digest, with null ActionIds and an explicit
historical-only marker. Physical work is not resampled and quiescence stays
unknown. Archived obligations expose their original requirements without
inventing a retained monitor or a next sample tick. No poll or zero-tick wait
is used to manufacture readable proof. Complete history counts and bytes are
admitted even when an output page is small.

Previously verified history remains inspectable after a later in-memory restore
or withdrawal, without renewing pursuit eligibility. The existing verifier does
not capture fresh action history for an original already abandoned or withdrawn
before reopening. Those cases, legacy missing maps and invalid captured custody
remain explicitly unavailable or unverifiable, with no inferred completion,
nondispatch or current physical quiet.

## Verification

Two new durable public-facade regression functions cover completion followed by
compaction and two reopens, exact typed-object mismatch, current Query denial,
resource and Explain memory/file purity, immutable historical proof after
restore, stale resource pins, refused resumed waiting and explicitly unavailable
history after a later abandoned reopen. The existing active-recovery test now
requires one archived completed pause plus one carried active dig, without
duplicates. Together with the first resource slice there are eleven new
MCP resource regression functions. Two independent source reviews found no
remaining blocker; scoped formatting and whitespace checks passed.

The actual pinned MCP test build first stopped on disk exhaustion in rustix.
After reclaiming only task-owned completed build outputs, it passed that point
but the asupersync compiler was killed twice, including a retry with serialized
compiler scheduling. No application tests ran. The locked offline metadata-only check was also
killed in that same dependency, before application type diagnostics. Application
compilation, executed MCP tests and full-runtime qualification therefore remain
unverified. The cause of SIGKILL was not established. Dependency pins and production features are intact.

The separate terrain/blueprint changes pass 476 scoped actual-source tests:
158 world library, 306 intent library and twelve blueprint integrations. That
evidence does not establish MCP, release, native/live or current-latest-nightly
qualification.
