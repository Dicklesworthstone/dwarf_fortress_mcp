# Spatial/1.8 retained-capture lifecycle

The spatial/1.8 native reader now invalidates retained captures at **map as well
as world load/unload**, and treats first-page response construction as a cache
publication transaction. This complements explicit Rust-side source recovery:
reconnecting must not revive a retained capture from an earlier map incarnation.
The native protocol remains 1.8 with exactly `Handshake` and `ReadObservation`.
No mutation or production admission is introduced.

## Map-incarnation fencing

`SC_MAP_LOADED`, `SC_MAP_UNLOADED`, `SC_WORLD_LOADED` and `SC_WORLD_UNLOADED` all
clear the retained cache and advance the bridge generation. An existing token
cannot continue after any of those boundaries. Unrelated state events do not
invalidate immutable pages. Saturated generation remains saturated and refuses
handshake/read admission; it never wraps and reuses an old generation.

The token counter still remains monotonic across cache clearing. These are local
bridge-incarnation fences, not evidence of global event-history continuity,
cryptographic randomness, a durable anti-rollback floor or external-controller
coordination. The Rust normalizer's existing generation-change handling remains
responsible for publishing an observation epoch reset.

## DFHack version API compatibility

The producer now uses the actual `const char*` return type of
`Version::dfhack_version()` separately from the `std::string` game version.
Its previous combined `const auto` declaration required both initializers to
have the same type and did not compile against that real signature. The old
string-returning mock masked the error. A null version pointer is also rejected
rather than passed to a string constructor. The primary declaration was checked
in `DFHack/dfhack:library/include/PluginManager.h` (blob
`1eccc7a79a88e8512c76db723519cf81e62baab1`); the map-event names were checked in
`library/include/CoreDefs.h` (blob `dd7864939409374c7758e6f7308e14278184524d`).
This source inspection is not a named-version native build.

## Exception containment and publication

Both handlers catch failures during authorization/version lookup as well as
capture and reply construction. Standard and non-standard C++ exceptions produce
an unsuccessful, cleared response. Private exception messages are not exposed.
If even the bounded error envelope cannot be constructed, the handler clears it
and reports RPC failure to DFHack rather than returning a partial success.

When a call creates a new retained capture, failure to construct its first page
removes only that newly inserted capture and restores its retained-byte budget.
Repeated failed first-page construction therefore cannot exhaust all four cache
slots. A later-page response failure does **not** remove an already acknowledged
capture: the original token can still resume immutable bytes with the original
digest, without another native capture.

The shared cache also performs no allocation after inserting a completed entry.
Its output token now uses a nonthrowing string swap. Previously, allocation failure
in the final token copy left an entry present even though insertion threw before
returning a handle. Allocations earlier in insertion retain the standard strong
container guarantees: no new cache entry is exposed on failure. Token counters
may advance on a failed attempt and are deliberately not rolled back.

A locally successful response whose transport acknowledgement is lost can still
leave a retained capture until normal release/expiry. This change does not guess
whether a client received a token, retire another caller's valid capture, or add
a bulk cache-reset RPC. Existing owner, nonce, region, citizen limit, acquisition
limit, page size, expiry and aggregate-memory bounds remain in force.

The shared cache change applies to its existing callers without changing token
bytes, SHA-256, payload bytes or paging semantics. The map-event and handler
transaction changes are specifically wired into spatial/1.8. Older native profile
handlers are not claimed to have acquired those new fences.

## Executed evidence

```bash
python scripts/test_spatial_capture_lifecycle.py --ubsan
python scripts/test_spatial_capture_lifecycle.py --compiler clang++ --ubsan
```

Both runs passed **237 C++ assertions across seven groups**, using C++17,
`-Wall -Wextra -Werror -pedantic -O1`, UBSan and nonrecovering sanitizer failures.
Groups cover map/world lifecycle, failed new replies, resumable failed
continuations, exception containment, ownership/protocol/bounds, generation
exhaustion, and every observed allocation point during cache insertion. The
allocation injector is test-only in a separate translation unit; production
allocation behavior is not replaced. Python independently checks the native
SHA-256 of the 40,000-byte paginated fixture.

Exact executed producer SHA-256:
`dc8502ab776cb034b11bf74ff0da2e3a6bba5f181e35bf5eb2fe3db9c936cc37`.
Exact executed cache-header SHA-256:
`76b91a4e5e9932a119f1458ee943cd3924148d2f4f6a219a98bed528fc44798a`.

The prior exact producer failed compilation against the corrected version-return
signature. With explicit `--legacy-version-double` only for historical producer
behavior probes, the same harness independently reproduced three more defects:
map events did not advance generation; failed first-page construction stranded
cache entries; version lookup exceptions escaped the native handler. Separately, running the
current producer with the exact old cache reproduced final token allocation
failure leaving an unreachable retained entry. That cache probe uses the corrected
version signature. Five independent negative probes therefore cover the compile
blocker plus four runtime defects. Use `--source`,
`--cache-source` and `--group` to run those regression probes against prior files.
The old producer/cache blobs were verified before editing as
`093f8492e831bc9a0c1a5894bf73d0385ba6b452` and
`5b5ef3ac696e931923916d30f2384d1bb43ddb8b` respectively.

The harness compiles the **actual complete RPC handler source and actual retained
cache header**, copied byte-for-byte. DFHack/protobuf interfaces and native field
capture are explicit doubles. Thus this evidence executes the auth, ownership,
pagination, digest, cache and handler-lifecycle code, but not the real native
citizen/operations/terrain field acquisition, generated protobuf runtime, actual
DFHack event delivery, Rust/MCP integration or a live game. The inherited
whole-capture mock now includes the additional map event symbols and corrected
version-return signature; its full suite
was not executed here. This is not native qualification or repository-wide
qualification. Compatibility registry, production runner map and admission
requirements remain unchanged.
