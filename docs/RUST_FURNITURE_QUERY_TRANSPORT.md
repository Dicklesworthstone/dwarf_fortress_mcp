# Original furniture receipt recovery through the Rust transport

The journal and session can now request exact original receipt reconciliation,
but the old Rust transport still rejected it: a native uncertainty flag could
never clear within a connection, and every observed Indeterminate receipt was
immutable even on an explicit recovery query. This change connects the existing
native QueryPlacement proof to the already-present session/MCP recovery path.

Only QueryPlacement with a fully decoded Placed receipt for the exact requested
plan and original native generation can explain a true-to-false native fence.
Handshake, observation, prepare, commit, cancellation, absent records and bare
flags cannot clear it. The existing full decoder still validates expected-after
capture equality, insertion links, record commitment and all canonical fields.

A previously seen Indeterminate record may advance to Placed only on that same
query path, with the original plan/generation and native unresolved=false. Normal
BuildRecord::validate_successor remains unchanged: no generic relaxation of
receipt immutability, new key, cancellation or inferred nonapplication is allowed.
Previously resolved records remain immutable. If successor or retention checks
fail, the connection is fenced and its previous validated native summary and
seen record are preserved, rather than publishing an untrusted cleared fence.

Recovery connections remain read-only, carry no initial capture or fresh prepare
permit, and cannot regain one from imported or recovered evidence. The original
journal independently checks source identity and appends/synchronizes positive
proof before returning it. Transport acceptance alone is not durable resolution,
construction completion, current usability or production admission.

The change adds no RPC, wire field, capability, dependency or protocol generation.
Concurrent journal, session and MCP reconciliation implementations are retained.
The machine contract is architecture/rust_furniture_query_recovery_v1.json.
Beads df-dfhack-bridge-plane-c-pic.4/.5 continue to own broader qualification.

## Validation status

Six Rust regression functions are registered. Two enumerate receipt-successor
and proof-gate cases using the actual canonical decoder. Four drive the actual
TCP client with bounded, joined loopback peers, covering first-query recovery,
same-connection recovery, repeated positive history, malformed clearance,
wrong generation, contradictory successor summaries, cancellation and inability
to prepare after recovery. Existing tests retaining a true uncertainty fence are
not weakened.

These tests have NOT been compiled or executed in the editing environment:
rustc, cargo and rustfmt are unavailable. Original/uploaded Git blob identities
were compared; that verifies publication bytes, not Rust correctness. No real
DFHack SDK, generated-protobuf, live-fortress or full-workspace qualification is
claimed. Run the normal repository qualification and targeted adapter tests in
the configured latest-nightly dependency-resolved workspace before admission.
