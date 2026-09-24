# Obligation observation correctness

- Make rejected obligation observation batches atomic rather than partially applying
  earlier actions before detecting a later action's regressed tick.
- Do not suppress failure or contradictory evidence behind polling cadence; reserve
  cadence and distinct-tick checks for positive stability accumulation.
- Fail an incomplete stability window at the exact deadline, not a later observation.
- Reject cancellation timestamps older than the last scheduled evaluation.
- Add ten regression tests for `df-action-coordinator-exec-ero.4`; these tests are
  uncompiled and unexecuted in the editing environment, not qualification evidence.
