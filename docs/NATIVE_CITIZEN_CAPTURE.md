# Native citizen capture integrity

The spatial/1.8 citizen component in `bridge/common/citizen_capture.h` now
preserves Dwarf Fortress text correctly and refuses malformed strict-roster or
skill observations before the caller can publish a successful combined capture.
This changes the data producer, not the protocol, native methods, capabilities,
production runner map, or empty compatibility registry.

## Corrected behavior

Dwarf Fortress strings are CP437. Visible translated names and readable race
names now pass through DFHack's `DF2UTF` declaration in `MiscUtils.h` before the
existing UTF-8 prefix bound. Previously, treating the raw CP437 bytes as UTF-8
silently truncated a name such as `Urist élan` at its first extended byte.
The raw input to conversion is limited first: at most 256 bytes for a name and
128 for a race. Every CP437 byte contributes at least one UTF-8 output byte, so
this cannot discard a scalar that would fit in the output limit. The output
still ends at a complete UTF-8 scalar. This bounds the conversion's additional
allocation, not allocations performed internally by DFHack when returning text.
Names remain bounded display prefixes, not a claim to preserve unbounded text.

Null pointers in a returned strict-citizen roster are now rejected before sorting
rather than erased and reported as a smaller complete population. Existing
count, identity, duplicate, membership, profession and stress checks remain in
force. A malformed member does not manufacture an absence certificate.

Negative nominal skill, effective skill, or experience is refused before sparse
zero-skill omission. Only three exactly zero values omit the skill. This prevents
both silently treating invalid negative records as unskilled citizens and
serializing mixed negative/positive records that downstream validation cannot
accept. Experience-only and rusted skills remain represented. Per-citizen and
global skill ceilings are unchanged.

The UTF-8 helper handles out-of-range offsets before indexing. Ambiguous one-line
`if`/following statements that failed GCC's warning-denied build were separated.
The citizen wire magic, field order, byte lengths, record ordering, and caller
error codes remain unchanged. As before, a nonzero capture status means the caller
must discard the output; a partial buffer is not a published observation.

## Executed regression evidence

Run the checked-in harness with either compiler:

```bash
python scripts/test_citizen_capture_codec.py --compiler g++ --ubsan
python scripts/test_citizen_capture_codec.py --compiler clang++ --ubsan
```

Both executions passed **339 C++ assertions across six groups**, under C++17,
`-Wall -Wextra -Werror -pedantic -O1`, with undefined-behavior sanitization and
nonrecovering sanitizer failures. Groups cover CP437 extended bytes, scalar and
allocation bounds, strict roster integrity/order, sparse/invalid skills and both
skill ceilings, request/output bounds, and UTF-8 scalar validation. Python also
independently checks the fixture's framing, ID and decoded UTF-8 name. Both
compilers produced the same fixture digest.

Executed citizen-header SHA-256:
`d470152439a22a2040ef38077d63e2487173370cafa2fabd7cf0706174ab92d7`.
Fixture SHA-256:
`434e6d45c7a5895d0999de2cc1a948800c9e2fce3a1a8b048c86213e7891df61`.

The exact prior header (Git blob `f4a8dea2e49831e97f24b39a859796a9591c2358`)
failed the warning-denied build. With only `misleading-indentation` demoted for
regression probes, its `encoding`, `roster`, and `skills` groups independently
failed on the intended truncated-name, null-member omission, and negative-skill
omission assertions. The harness accepts `--source` and `--group` to reproduce
those probes without editing the production header.

These executions compile the **actual citizen codec body**, copied byte-for-byte,
with explicit DFHack and serialization boundary doubles. They do not test the
real `DF2UTF` implementation, generated DF/protobuf headers, the whole spatial RPC
producer, Rust decoding, MCP, a running game, or repository-wide qualification.
The pre-existing whole-spatial-producer harness was updated with an explicitly
ASCII-only converter double; that small adapter was compiled on both compilers,
but the whole producer harness was not executed in this environment. Extended
character tests use the dedicated codec harness instead of an identity converter.
Rust, Cargo, and rustfmt remain unavailable; no admission claim follows.
