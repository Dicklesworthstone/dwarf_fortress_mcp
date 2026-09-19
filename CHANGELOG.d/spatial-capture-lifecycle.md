## Spatial/1.8 map fencing and transactional retained captures

- Invalidate retained native pages on map load/unload in addition to world
  load/unload, with monotonic generation and fail-closed exhaustion.
- Fix a real DFHack API compile blocker: use the const-char-pointer version
  declaration rather than the string-returning double that masked mixed-auto
  deduction. Reject null version pointers; align the existing spatial mock.
- Contain authorization, capture and reply-construction exceptions in both
  native handlers. Never emit partial accepted payloads or private exception text.
- Roll back a newly retained capture when its first response cannot be built,
  while keeping already acknowledged captures resumable after later-page failure.
- Eliminate the shared cache's post-publication token allocation failure using
  a nonthrowing swap. Keep token/wire bytes, existing method set and limits.
- Execute 237 actual-handler/cache assertions on GCC and Clang with UBSan and
  warning-denied C++17 builds; reproduce five independent failures in prior source.
  DFHack/protobuf/field acquisition are explicit doubles, not live qualification.

See `docs/SPATIAL_CAPTURE_LIFECYCLE.md` for exact source hashes, commands and limits.
No native protocol, game effect, compatibility entry or production runner change.
