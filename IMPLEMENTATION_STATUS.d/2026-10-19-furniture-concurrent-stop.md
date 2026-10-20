## Concurrent original furnishing stop publication

Beads: df-dfhack-bridge-plane-c-pic.4/.5;
df-cancellation-progress-certificates-vvw. Broader execution/drain work remains open.

The ordinary furnishing stop command needs the batch root lock; an active
placement or final publication holds that lock. The new operator-only
request_furniture_batch_stop.py records the same permanent marker without
root, manifest, index, effect or external terrain locks. It reuses the unchanged
no-follow/pinned file primitives of request_dig_blueprint_stop.py, without
monkeypatching globals or changing the dig command's semantics.

Identity-only verification binds the exact SHA-256 of the sealed original
manifest, all four profile-specific bounds, root/effects directory identities
and a descriptor-pinned live index. Index contents may append while the signal
is published; no index snapshot or effect inventory is claimed. Matching full
markers can be re-synced unchanged after lost acknowledgment; torn/conflicting
markers refuse unchanged. Stop success never claims native cancellation, game
pause, quiescence, rollback, completion, new authority or commit retry.

Executed 27 warnings-denied Python tests on Linux using real files, locks,
fsync calls, subprocesses and SIGKILL. Coverage includes a CLI stop with all
executor-like locks held in another live process, live index growth, every
checkpoint of a small publication, both fsync acknowledgment losses, partial
writes/process death, concurrent marker owners, source/marker/root replacement,
profile byte/nesting bounds, special files/links, descriptor cleanup and one
stdout attempt. Unsupported platforms refuse before importing POSIX locking.

The shared unchanged helper was fetched from main b8802744 and checked against
Git blob 392872ecb63eae9d3c748ceaf5328f0aac569138. The test manifests are
identity-only synthetic data, not semantic allocation/room/terrain evidence.
Existing furniture Batch stop decoding, membership checks and per-dispatch guards
were source-reviewed at blob bbbc094e0b59cacaf556398900f0a8f061a10635. The
prefix owner at blob 723af826a0f75613f09cb743ccce780c4b3e29cc was also read.
Their complete native-batch integration was not executed. No AST-extracted
runtime or replacement placement client was presented as integration evidence.

Native/TCP placement, full journal recovery, room/terrain validation, Rust,
DFHack, full-repository and live-fortress qualification were not run. Existing
native methods, dispatchers, journal formats, effect keys, recovery commands
and production admission remain unchanged. This fragment records local source
and component evidence; it does not itself attest remote publication.

### Publication revalidation

Reapplied the pending implementation onto upstream 49769e29 without altering
its independently added bounded-cost corridor work. The shared stop helper
remains blob 392872ecb63eae9d3c748ceaf5328f0aac569138. All 27 warnings-denied
component/CLI cases passed again on the publication files. Uploaded runtime
and test blobs match the executed bytes. Complete Batch/native integration is
still not claimed by this initial publication.
