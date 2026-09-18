# Native pause preparation fencing

The control/1.7 bridge previously checked token identity and a nonregressed game
clock, but did not fence a preparation after a different pause setter. An older
prepared unpause could therefore overwrite a newer pause, even at the same tick.
A no-op pause setter could also leave every older preparation dispatchable.

The native handler now captures an in-process dispatch sequence, observed pause
state, game tick and monotonic preparation time. Immediately before the first
setter it requires the same sequence and pause state, a nonregressed tick and
less than 60 seconds of monotonic age. A successful claim advances the sequence
**before** calling the setter. Other preparations are stale even if that setter
fails, has an ambiguous result, does nothing, or a later effect restores the
original pause state. Known stale preparations are permanently retired.

Prepare replay retains its original token, guard and creation time. It does not
renew the lifetime. Already requested operations still return historical evidence
without calling the setter or rechecking preparation age. Receipt-less requested
records remain indeterminate and cannot redispatch. Terminal receipt bytes and
prepare-token derivation are unchanged.

A current pause-state mismatch, clock failure/regression or unavailable fortress
retires the preparation. Map load/unload now advances bridge generation and clears
records, just as world load/unload does, so a new local map cannot inherit old
preparations or receipt lookups. Generation exhaustion refuses authentication
rather than wrapping. The internal dispatch sequence also cannot wrap.

The handler catches setter/readback exceptions. An exception after the setter
attempt leaves a requested record with no terminal receipt; it cannot escape that
native effect section or enable retry. A fully observed setter failure still gets
the existing not-applied receipt. The wire's existing `query_only=true` restriction
now refuses both preparation and commit; it does not replace bearer authentication.

## Scope and compatibility

This is stricter preflight inside the existing unadmitted control/1.7 family, not a
new effect or a protocol-generation expansion. The four RPC names, protobuf fields,
token/receipt encodings, Rust trust domain and durable journal remain unchanged.
No capability, production runner or compatibility registry is widened.

DFHack's ordinary suspended RPC execution must serialize these handlers. The fence
is local to this plugin incarnation: it does not lock the UI, other plugins or
independent controllers. It detects a currently changed external pause state, not
an external change-and-restore that went unobserved. It is not a global lease or
proof of absence of all interference.

The existing Rust coordinator writes CommitStarted before sending. A native stale
rejection does **not** manufacture a new durable not-applied receipt. When no valid
terminal receipt is available, reconciliation remains conservative. Never retry an
unresolved commit under the same key. A new effect requires fresh preparation and
its own identity; old keys remain retained within the existing 4,096-record bound.

The preparation timeout limits how long a token may start an effect. It does not
bound how long the game runs after an unpause. Autonomous bounded simulation still
needs a separate native watchdog and reviewed protocol, not a client-side sleep.

## Executed regression evidence

`python3 scripts/test_native_control.py --compiler g++ --sanitize --mutation-check`
compiles the **actual production handler source** with explicit DFHack/protobuf
boundary doubles, then executes ten grouped scenarios covering conflicting
preparations, same-tick ABA, no-op setters, monotonic expiry, changed preconditions,
setter/readback faults, observed non-application, duplicate historical receipts,
map/world resets and read-only/authentication/identity checks. The clock tests also
exercise extreme time points without signed duration overflow.

GCC and Clang builds passed with C++17, -Wall -Wextra -Werror -pedantic and undefined
behavior sanitization. An optimized GCC build also passed. Removing the dispatch
gate made the expected stale-preparation regression fail under both compilers.

A separate before/after execution compiled the upstream handler blob
`1012c2a062bd251595e87df2ceb4eb93b4a5f9bb` and the modified handler against the same
boundary doubles: the upstream handler accepted the stale unpause, called the
setter twice and ended unpaused; the modified handler rejected it, called the
setter once and retained the newer pause.

These are executed C++ handler tests, not independent Python algorithms. They are
still **not** a real DFHack SDK/plugin build, protobuf-runtime test, live fortress
campaign, Rust compilation, full-repository qualification or admission evidence.
The test runner emits SHA-256 values of the exact source and test files it uses.
