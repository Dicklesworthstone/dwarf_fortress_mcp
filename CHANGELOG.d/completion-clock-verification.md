## Reach original construction stability after quiescence

- Add separately confirmed bounded clock advancement for an active original
  completion monitor whose full construction condition is already quiescent.
- Bind exact original goal, journal head, timing and remaining samples to the
  fresh clock/capture. Keep original batch and completion journal read-only.
- Use a distinct frv-* scope with the existing prior-history fence, synchronized
  intent, effect guards and original-key recovery. Never fabricate a monitor
  sample, change its deadline, claim completion or automatically loop.
- Preserve old next-step/final scopes and classify new verification records in
  shared discovery, with the correct independent recovery entrypoint.
- All 35 Python tests and seven assertion-based mutation checks pass. Workflow
  codecs, native acquisition/transport and original file custody are explicit
  test ports; no native/live/full-workspace qualification is claimed.
